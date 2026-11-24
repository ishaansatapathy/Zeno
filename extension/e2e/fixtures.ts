import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { chromium, expect, test as base, type BrowserContext, type Page, type Worker } from "@playwright/test";

import { developmentExtensionId } from "../scripts/extension-id.mjs";

const DIST = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "dist");

export interface Stored {
  id: string;
  files: { name: string; size: number }[];
  manifest: {
    status: string;
    tracks: { track: string; file: string; mime: string }[];
    durationMs?: number;
    pauses?: { from: number; to: number | null }[];
    joints?: { at: number; ms: number }[];
    recovered?: boolean;
  } | null;
  upload: { status: string; partSize: number } | null;
}

interface Fixtures {
  context: BrowserContext;
  extensionId: string;
  worker: Worker;
}

export const test = base.extend<Fixtures>({
  // A fresh browser profile with the extension loaded and fake devices, for every test.
  context: async ({}, use) => {
    const profile = fs.mkdtempSync(path.join(os.tmpdir(), "zeno-extension-"));
    // Recording the screen needs a screen, so those tests (ZENO_SCREEN=1) run in a visible window and let
    // the browser pick the screen for itself instead of showing its own picker.
    const screen = Boolean(process.env["ZENO_SCREEN"]);
    const context = await chromium.launchPersistentContext(profile, {
      channel: "chromium",
      headless: !screen,
      args: [
        `--disable-extensions-except=${DIST}`,
        `--load-extension=${DIST}`,
        "--use-fake-device-for-media-stream",
        "--use-fake-ui-for-media-stream",
        ...(screen ? ["--auto-select-desktop-capture-source=Screen"] : []),
      ],
    });
    await use(context);
    await context.close();
    fs.rmSync(profile, { recursive: true, force: true });
  },
  worker: async ({ context }, use) => {
    // The listener goes first: a worker that starts between looking and listening would be missed for good.
    const starting = context.waitForEvent("serviceworker", { timeout: 20_000 }).catch(() => null);
    let found = context.serviceWorkers()[0] ?? (await starting);
    if (!found) {
      // The browser stops an idle worker, and one that had already stopped never announces itself again. Any
      // message to the extension wakes it, and the popup sends one as soon as it opens.
      const waking = context.waitForEvent("serviceworker", { timeout: 20_000 });
      const page = await context.newPage();
      await page.goto(`chrome-extension://${developmentExtensionId()}/popup.html`);
      found = await waking;
      await page.close();
    }
    await use(found);
  },
  extensionId: async ({ worker }, use) => {
    await use(new URL(worker.url()).host);
  },
});

export { expect };

/**
 * The popup, opened as a page so a test can drive it. The countdown is off unless a test asks for it, so that a
 * test about something else does not wait three seconds to start.
 */
export async function openPopup(context: BrowserContext, extensionId: string, options: { countdown?: boolean } = {}): Promise<Page> {
  const page = await context.newPage();
  await page.goto(`chrome-extension://${extensionId}/popup.html`);
  const countdown = options.countdown ?? false;
  await page.evaluate(async (wanted) => {
    const kept = await chrome.storage.local.get("settings");
    await chrome.storage.local.set({ settings: { ...(kept["settings"] as object | undefined), countdown: wanted } });
  }, countdown);
  await page.reload();
  return page;
}

/** What the toolbar icon says. */
export async function badge(worker: Worker): Promise<string> {
  return worker.evaluate(() => chrome.action.getBadgeText({}));
}

/**
 * Does what a person does the first time: opens the page that asks for the camera and microphone and
 * says yes (the browser's fake prompt answers for the click), so the popup can then record.
 */
export async function allowDevices(context: BrowserContext, extensionId: string): Promise<void> {
  const page = await context.newPage();
  await page.goto(`chrome-extension://${extensionId}/permissions.html?microphone=true&camera=true`);
  await page.getByRole("button", { name: "Continue" }).click();
  await expect(page.locator("#message")).toContainText("Done");
  await page.close();
}

/** What the extension has on disk, read from inside the extension (its storage is its own). */
export async function stored(page: Page): Promise<Stored[]> {
  return page.evaluate(async () => {
    const root = await navigator.storage.getDirectory();
    const result: Stored[] = [];
    let recordings: FileSystemDirectoryHandle;
    try {
      recordings = await root.getDirectoryHandle("recordings");
    } catch {
      return result;
    }
    for await (const [id, handle] of (recordings as unknown as { entries(): AsyncIterable<[string, FileSystemHandle]> }).entries()) {
      const files: Stored["files"] = [];
      let manifest: Stored["manifest"] = null;
      let upload: Stored["upload"] = null;
      for await (const [name, entry] of (handle as unknown as { entries(): AsyncIterable<[string, FileSystemHandle]> }).entries()) {
        const file = await (entry as FileSystemFileHandle).getFile();
        files.push({ name, size: file.size });
        if (name === "manifest.json") manifest = JSON.parse(await file.text());
        if (name === "upload.json") upload = JSON.parse(await file.text());
      }
      result.push({ id, files, manifest, upload });
    }
    return result;
  });
}

/** Copies one stored file out to a real file on disk, so ffprobe can read it. */
export async function copyOut(page: Page, id: string, name: string): Promise<string> {
  const base64 = await page.evaluate(
    async ([recording, file]) => {
      const root = await navigator.storage.getDirectory();
      const directory = await (await root.getDirectoryHandle("recordings")).getDirectoryHandle(recording as string);
      const bytes = new Uint8Array(await (await (await directory.getFileHandle(file as string)).getFile()).arrayBuffer());
      let binary = "";
      for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
      return btoa(binary);
    },
    [id, name],
  );
  const target = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "zeno-file-")), name);
  fs.writeFileSync(target, Buffer.from(base64, "base64"));
  return target;
}

export interface Probe {
  video: { codec: string; width: number; height: number } | null;
  audio: { codec: string } | null;
  seconds: number;
}

/** What ffprobe says a file is. Throws if it is not a media file at all. */
export function probe(file: string): Probe {
  const run = spawnSync("ffprobe", ["-v", "error", "-show_streams", "-show_format", "-of", "json", file], { encoding: "utf8" });
  if (run.status !== 0) throw new Error(`ffprobe could not read ${file}: ${run.stderr}`);
  const info = JSON.parse(run.stdout) as { streams: { codec_type: string; codec_name: string; width?: number; height?: number; duration?: string }[]; format: { duration?: string } };
  const video = info.streams.find((s) => s.codec_type === "video");
  const audio = info.streams.find((s) => s.codec_type === "audio");
  // WebM written live by a MediaRecorder has no length in its header (ffprobe says N/A), so the length is
  // read from the last picture in the file instead.
  let seconds = Number(info.format.duration ?? video?.duration ?? audio?.duration ?? 0);
  if (!Number.isFinite(seconds) || seconds <= 0) {
    const packets = spawnSync("ffprobe", ["-v", "error", "-select_streams", "v:0", "-show_entries", "packet=pts_time", "-of", "csv=p=0", file], { encoding: "utf8" });
    const times = packets.stdout.split(/\r?\n/).map(Number).filter(Number.isFinite);
    seconds = times.length ? Math.max(...times) : 0;
  }
  return {
    video: video ? { codec: video.codec_name, width: video.width ?? 0, height: video.height ?? 0 } : null,
    audio: audio ? { codec: audio.codec_name } : null,
    seconds,
  };
}
