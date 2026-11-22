// The main-thread side of the disk writer, and the small files that describe a recording.

type Reply = { n: number; ok: true; size: number; data?: ArrayBuffer } | { n: number; ok: false; name: string; message: string };

export class DiskError extends Error {
  constructor(
    readonly kind: string,
    message: string,
  ) {
    super(message);
  }
}

/**
 * One file on disk, written chunk by chunk in order. Writes are queued so they cannot overtake one
 * another, and a failed write is remembered: every later call reports it, so a full disk is noticed at
 * the next chunk and the next stop, not only if somebody looks.
 */
export class DiskWriter {
  private n = 0;
  private waiting = new Map<number, { resolve: (reply: { size: number; data?: ArrayBuffer }) => void; reject: (error: DiskError) => void }>();
  private bytes = 0;
  private closing = false;
  private settle: () => void = () => undefined;
  /** Settles once the file has been closed: after that it can be read as a plain file. */
  readonly closed = new Promise<void>((resolve) => {
    this.settle = resolve;
  });
  private tail: Promise<unknown> = Promise.resolve();
  private failure: DiskError | null = null;

  private constructor(private readonly worker: Worker) {
    worker.onmessage = (event: MessageEvent<Reply>) => {
      const reply = event.data;
      const pending = this.waiting.get(reply.n);
      this.waiting.delete(reply.n);
      if (!pending) return;
      if (reply.ok) return pending.resolve(reply.data ? { size: reply.size, data: reply.data } : { size: reply.size });
      const error = new DiskError(reply.name, reply.message);
      this.failure ??= error;
      pending.reject(error);
    };
  }

  /** Opens `path` (directories first, then the file) for writing, creating it new. */
  static async open(path: string[]): Promise<DiskWriter> {
    const writer = new DiskWriter(new Worker(chrome.runtime.getURL("writer.worker.js")));
    await writer.send({ op: "open", path });
    return writer;
  }

  private send(request: { op: "open"; path: string[] } | { op: "write"; data: ArrayBuffer } | { op: "read"; start: number; end: number } | { op: "close" }): Promise<{ size: number; data?: ArrayBuffer }> {
    const n = (this.n += 1);
    return new Promise((resolve, reject) => {
      this.waiting.set(n, { resolve, reject });
      this.worker.postMessage({ n, ...request }, request.op === "write" ? [request.data] : []);
    });
  }

  /** Queues a chunk. Resolves once it has been written and flushed. */
  write(chunk: Blob): Promise<void> {
    const next = this.tail.then(async () => {
      if (this.failure) throw this.failure;
      const { size } = await this.send({ op: "write", data: await chunk.arrayBuffer() });
      this.bytes = size;
    });
    this.tail = next.catch(() => undefined);
    return next;
  }

  /** Waits for everything queued, closes the file, and says how big it is. */
  async close(): Promise<number> {
    this.closing = true;
    await this.tail;
    if (this.failure) {
      this.worker.terminate();
      this.settle();
      throw this.failure;
    }
    const { size } = await this.send({ op: "close" });
    this.worker.terminate();
    this.settle();
    return size;
  }

  /** Bytes that have been written and flushed. */
  get size(): number {
    return this.bytes;
  }

  /**
   * Reads a range that has already been flushed, while the recording goes on. It waits its turn behind the
   * writes, so it can never see half a chunk, and it never reaches a file that is being closed: it says so
   * (kind "Closed"), and the caller reads the finished file instead.
   */
  read(start: number, end: number): Promise<ArrayBuffer> {
    const next = this.tail.then(async () => {
      if (this.closing) throw new DiskError("Closed", "The file has been closed.");
      const { data } = await this.send({ op: "read", start, end });
      if (!data) throw new DiskError("Error", "The recording could not be read.");
      return data;
    });
    this.tail = next.catch(() => undefined);
    return next;
  }

  get failed(): DiskError | null {
    return this.failure;
  }
}

async function directory(path: string[], create: boolean): Promise<FileSystemDirectoryHandle> {
  let current = await navigator.storage.getDirectory();
  for (const part of path) current = await current.getDirectoryHandle(part, { create });
  return current;
}

/** Writes a small JSON file in one go (the manifest of a recording). */
export async function writeJson(path: string[], value: unknown): Promise<void> {
  const file = path[path.length - 1];
  if (!file) throw new Error("no file name");
  const handle = await (await directory(path.slice(0, -1), true)).getFileHandle(file, { create: true });
  const writable = await handle.createWritable();
  await writable.write(JSON.stringify(value, null, 2));
  await writable.close();
}

export async function readFile(path: string[]): Promise<File> {
  const file = path[path.length - 1];
  if (!file) throw new Error("no file name");
  return (await (await directory(path.slice(0, -1), false)).getFileHandle(file)).getFile();
}

export async function removeDirectory(path: string[]): Promise<void> {
  const name = path[path.length - 1];
  if (!name) return;
  try {
    await (await directory(path.slice(0, -1), false)).removeEntry(name, { recursive: true });
  } catch {
    /* already gone */
  }
}
