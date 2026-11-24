// The recordings kept in the extension's own storage, read and written from any part of the extension (the
// service worker, the offscreen document, a popup): they all share one private file system. What a recording
// is, and what to do about it, is decided by `recovery.ts`; this only touches the disk.

import { removeDirectory, writeJson } from "../offscreen/opfs";
import type { Manifest, Stored, UploadFile } from "./recovery";

async function recordings(create = false): Promise<FileSystemDirectoryHandle | null> {
  try {
    return await (await navigator.storage.getDirectory()).getDirectoryHandle("recordings", { create });
  } catch {
    return null;
  }
}

async function readJson<T>(directory: FileSystemDirectoryHandle, name: string): Promise<T | null> {
  try {
    const file = await (await directory.getFileHandle(name)).getFile();
    return JSON.parse(await file.text()) as T;
  } catch {
    // Not there, or half written when the browser went: neither is something to stop at.
    return null;
  }
}

/** The file that is uploaded: the first track the manifest names, or failing that the first video file there is. */
async function primary(directory: FileSystemDirectoryHandle, manifest: Manifest | null): Promise<File | null> {
  const names: string[] = manifest?.tracks.map((track) => track.file) ?? [];
  if (names.length === 0) {
    for await (const [name] of (directory as unknown as { entries(): AsyncIterable<[string, FileSystemHandle]> }).entries()) {
      if (name.endsWith(".webm")) names.push(name);
    }
  }
  const [name] = names;
  if (!name) return null;
  try {
    return await (await directory.getFileHandle(name)).getFile();
  } catch {
    return null;
  }
}

async function describe(id: string, directory: FileSystemDirectoryHandle): Promise<Stored> {
  const manifest = await readJson<Manifest>(directory, "manifest.json");
  const upload = await readJson<UploadFile>(directory, "upload.json");
  const downloadIds = await readJson<Stored["downloadIds"]>(directory, "downloads.json");
  const file = await primary(directory, manifest);
  // Only sweep an empty recording when every expected track could be read and was empty.
  let emptyVerified = false;
  let bytes = file?.size ?? 0;
  let lastWriteAt = file?.lastModified ?? 0;
  if (!bytes && manifest?.tracks.length) {
    try {
      const files = await Promise.all(manifest.tracks.map(async ({ file }) => (await directory.getFileHandle(file)).getFile()));
      bytes = Math.max(...files.map((track) => track.size));
      lastWriteAt = Math.max(...files.map((track) => track.lastModified));
      emptyVerified = bytes === 0;
    } catch {
      // An unreadable track may still hold data.
    }
  }
  return { id, manifest, upload, bytes, lastWriteAt, emptyVerified, ...(downloadIds ? { downloadIds } : {}) };
}

export async function readStored(id: string): Promise<Stored | null> {
  const root = await recordings();
  if (!root) return null;
  try {
    return await describe(id, await root.getDirectoryHandle(id));
  } catch {
    return null;
  }
}

export async function listStored(): Promise<Stored[]> {
  const root = await recordings();
  if (!root) return [];
  const found: Stored[] = [];
  for await (const [id, handle] of (root as unknown as { entries(): AsyncIterable<[string, FileSystemHandle]> }).entries()) {
    if (handle.kind === "directory") found.push(await describe(id, handle as FileSystemDirectoryHandle));
  }
  return found;
}

/** The files of a recording, in the order of its manifest's tracks. */
export async function trackFiles(id: string, manifest: Manifest): Promise<{ track: "screen" | "camera"; name: string; mime: string; file: File }[]> {
  const root = await recordings();
  if (!root) return [];
  const directory = await root.getDirectoryHandle(id);
  const files = [];
  for (const track of manifest.tracks) {
    try {
      files.push({ track: track.track, name: track.file, mime: track.mime, file: await (await directory.getFileHandle(track.file)).getFile() });
    } catch {
      // A track whose file is gone is left out: what remains is still worth finishing.
    }
  }
  return files;
}

export async function saveManifest(id: string, manifest: object): Promise<void> {
  await writeJson(["recordings", id, "manifest.json"], manifest);
}

export async function saveUpload(id: string, upload: UploadFile): Promise<void> {
  await writeJson(["recordings", id, "upload.json"], upload);
}

export async function saveDownloads(id: string, ids: NonNullable<Stored["downloadIds"]>): Promise<void> {
  await writeJson(["recordings", id, "downloads.json"], ids);
}

export async function removeStored(id: string): Promise<void> {
  await removeDirectory(["recordings", id]);
}
