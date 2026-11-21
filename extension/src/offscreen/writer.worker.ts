// Writes a recording to disk as it is made.
//
// Chrome's private file system (OPFS) offers writes that are flushed to disk one by one only inside a
// worker, and flushing after every chunk is the whole point: a recording that is held in memory, or
// written in one go at the end, is lost with the browser or the machine. Written like this, a crash costs
// the last second, not the recording.

interface SyncHandle {
  write(buffer: ArrayBuffer | ArrayBufferView, options?: { at?: number }): number;
  flush(): void;
  getSize(): number;
  truncate(size: number): void;
  close(): void;
}

type Message =
  | { n: number; op: "open"; path: string[] }
  | { n: number; op: "write"; data: ArrayBuffer }
  | { n: number; op: "close" };

type Reply = { n: number; ok: true; size: number } | { n: number; ok: false; name: string; message: string };

let handle: SyncHandle | null = null;
let offset = 0;

async function open(path: string[]): Promise<void> {
  const file = path[path.length - 1];
  if (!file) throw new Error("no file name");
  let directory = await navigator.storage.getDirectory();
  for (const part of path.slice(0, -1)) directory = await directory.getDirectoryHandle(part, { create: true });
  const fileHandle = (await directory.getFileHandle(file, { create: true })) as FileSystemFileHandle & { createSyncAccessHandle(): Promise<SyncHandle> };
  handle = await fileHandle.createSyncAccessHandle();
  // A file that already has bytes in it is from an earlier attempt at the same name: start it again.
  handle.truncate(0);
  offset = 0;
}

self.onmessage = async (event: MessageEvent<Message>) => {
  const request = event.data;
  const reply = (r: Reply) => (self as unknown as { postMessage(message: Reply): void }).postMessage(r);
  try {
    if (request.op === "open") {
      await open(request.path);
      return reply({ n: request.n, ok: true, size: 0 });
    }
    if (!handle) throw new Error("the file is not open");
    if (request.op === "write") {
      const written = handle.write(request.data, { at: offset });
      if (written !== request.data.byteLength) throw new DOMException("The disk did not take the whole chunk.", "QuotaExceededError");
      offset += written;
      handle.flush();
      return reply({ n: request.n, ok: true, size: offset });
    }
    handle.flush();
    const size = handle.getSize();
    handle.close();
    handle = null;
    return reply({ n: request.n, ok: true, size });
  } catch (error) {
    const e = error as { name?: string; message?: string };
    reply({ n: request.n, ok: false, name: e.name ?? "Error", message: e.message ?? String(error) });
  }
};

