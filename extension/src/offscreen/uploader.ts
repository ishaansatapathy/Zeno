// Uploads a recording while it is being made.
//
// The file on disk is the truth and the upload only follows it: the recorder writes (and flushes) a chunk a
// second, and this reads the file in fixed-size parts and sends each part as soon as it is full, straight to
// storage (the URL for a part comes from the API, signed for exactly its length). Nothing here can slow the
// recording or lose it: if the network is slow, down or flaky, parts simply wait on disk and catch up later,
// and the person's own copy is saved either way.
//
// When the recording ends, only what has not been sent yet is left (less than one part), the size is stated,
// and the API checks what storage holds before it assembles anything. The share link is made at the START,
// not the end, so it is in hand the moment the person stops: the page behind it shows "being prepared"
// until the upload is finished and processed.

import { transient, type ApiFailure, type Backend } from "../shared/api";
import type { UploadView } from "../shared/messages";

/** The smallest part storage accepts for anything but the last. Small parts keep what is left at the end small. */
export const PART_BYTES = 5 * 1024 * 1024;

export interface Part {
  /** From 1. */
  number: number;
  start: number;
  /** Exclusive. */
  end: number;
  last: boolean;
}

/**
 * The next part to send, if there is one. `done` parts (the first `done`) are already in storage.
 * While recording, only a part that is full is sent. Once the recording has ended, what is left is sent too,
 * as the last part, however short.
 */
export function nextPart(size: number, partSize: number, done: number, ended: boolean): Part | null {
  const start = done * partSize;
  if (start + partSize <= size) return { number: done + 1, start, end: start + partSize, last: ended && start + partSize === size };
  if (ended && start < size) return { number: done + 1, start, end: size, last: true };
  return null;
}

/** What the uploader reads the recording from. */
export interface Source {
  /** Bytes that are safely on disk. */
  size(): number;
  read(start: number, end: number): Promise<ArrayBuffer>;
}

export interface Deps {
  backend: Backend;
  source: Source;
  /** Sends one part to its address. Throws if the storage did not take it. */
  put(url: string, body: ArrayBuffer): Promise<void>;
  sleep(ms: number): Promise<void>;
  emit(view: UploadView): void;
}

export interface Plan {
  /** Names the recording on the server, so that a repeated request makes nothing twice. */
  recordingId: string;
  title: string;
  filename: string;
  partSize?: number;
}

/** Waiting before the n-th retry: 0.5 s, 1 s, 2 s ... up to 15 s. */
export function backoff(attempt: number): number {
  return Math.min(500 * 2 ** attempt, 15_000);
}

/** How long to keep trying after the recording has ended, before saying it could not be uploaded. */
const GIVE_UP_AFTER_END_MS = 90_000;

class Fatal extends Error {
  constructor(
    message: string,
    readonly status = 0,
  ) {
    super(message);
  }
}

export class Uploader {
  private readonly partSize: number;
  private view: UploadView = { phase: "connecting", sent: 0, total: null, url: null, copied: false };
  private videoId: string | null = null;
  private done = 0;
  private endedAt: number | null = null;
  private total: number | null = null;
  private wake: (() => void) | null = null;
  private finished: Promise<UploadView> | null = null;

  constructor(
    private readonly deps: Deps,
    private readonly plan: Plan,
  ) {
    this.partSize = plan.partSize ?? PART_BYTES;
  }

  get current(): UploadView {
    return this.view;
  }

  /** Begins, and returns at once: the work goes on by itself. */
  start(): void {
    this.finished = this.run();
  }

  /** More of the recording is on disk. */
  notify(): void {
    this.wake?.();
  }

  /** The recording has ended and is `total` bytes. Resolves with how the upload ended. */
  end(total: number): Promise<UploadView> {
    this.total = total;
    this.endedAt = Date.now();
    this.show({ total });
    this.notify();
    return this.finished ?? Promise.resolve(this.view);
  }

  private show(change: Partial<UploadView>): void {
    this.view = { ...this.view, ...change };
    this.deps.emit(this.view);
  }

  private get ended(): boolean {
    return this.total !== null;
  }

  private expired(): boolean {
    return this.endedAt !== null && Date.now() - this.endedAt > GIVE_UP_AFTER_END_MS;
  }

  /** Runs `step` until it works. Refusals (a person who is signed out, a request the API will not take) end it. */
  private async persist<T>(step: () => Promise<T>, whileWaiting: (error: unknown) => void): Promise<T> {
    for (let attempt = 0; ; attempt += 1) {
      try {
        return await step();
      } catch (error) {
        if (error instanceof Fatal) throw error;
        if (this.expired()) throw new Fatal("Could not reach Zeno to finish the upload.");
        whileWaiting(error);
        await this.deps.sleep(backoff(Math.min(attempt, 5)));
      }
    }
  }

  /** Throws `Fatal` for a refusal, a plain error for anything that may work later. */
  private ok<T>(result: { ok: true; data: T } | { ok: false; error: ApiFailure }): T {
    if (result.ok) return result.data;
    if (transient(result.error)) throw new Error(result.error.message);
    throw new Fatal(result.error.message, result.error.status);
  }

  private async run(): Promise<UploadView> {
    try {
      if (!(await this.signedIn())) return this.view;
      await this.setUp();
      await this.sendParts();
      return this.view;
    } catch (error) {
      const message = error instanceof Fatal ? error.message : "The upload stopped.";
      // An account that may not upload (a student's) is not a failure to apologise for: it is just not uploaded.
      if (error instanceof Fatal && error.status === 403) this.show({ phase: "off", reason: "refused", message: "This account cannot upload recordings." });
      else this.show({ phase: "failed", message });
      return this.view;
    }
  }

  /** Is there a Zeno session to upload as? If there is none, the recording is only saved on this computer. */
  private async signedIn(): Promise<boolean> {
    for (let attempt = 0; ; attempt += 1) {
      const me = await this.deps.backend.me();
      if (me.ok) return true;
      if (me.error.status === 401 || me.error.status === 403) {
        this.show({ phase: "off", reason: "signed_out" });
        return false;
      }
      if (!transient(me.error)) {
        this.show({ phase: "off", reason: "refused", message: me.error.message });
        return false;
      }
      this.show({ phase: "connecting", reason: "unreachable" });
      if (this.expired()) throw new Fatal("Could not reach Zeno to upload this recording.");
      await this.deps.sleep(backoff(Math.min(attempt, 5)));
    }
  }

  /** The upload, the recording it belongs to, and the link. All three are safe to repeat. */
  private async setUp(): Promise<void> {
    const { backend } = this.deps;
    const started = await this.persist(
      async () => this.ok(await backend.startUpload({ filename: this.plan.filename, title: this.plan.title, partSize: this.partSize })),
      () => this.show({ phase: "connecting", reason: "unreachable" }),
    );
    this.videoId = started.video.id;
    await this.persist(
      async () => this.ok(await backend.createRecording({ id: this.plan.recordingId, title: this.plan.title, videoId: started.video.id })),
      () => this.show({ phase: "connecting", reason: "unreachable" }),
    );
    const link = await this.persist(
      async () => this.ok(await backend.createLink(this.plan.recordingId)),
      () => this.show({ phase: "connecting", reason: "unreachable" }),
    );
    this.show({ phase: this.ended ? "finishing" : "uploading", reason: undefined, url: link.url });
  }

  private async sendParts(): Promise<void> {
    const { backend, source } = this.deps;
    const videoId = this.videoId;
    if (!videoId) throw new Fatal("The upload was not started.");
    for (;;) {
      const part = nextPart(this.ended ? (this.total ?? 0) : source.size(), this.partSize, this.done, this.ended);
      if (!part) {
        if (this.ended) break;
        // Nothing full yet: wait for the recorder to write more (or for the end).
        await new Promise<void>((resolve) => {
          this.wake = resolve;
          setTimeout(resolve, 2000);
        });
        this.wake = null;
        continue;
      }
      await this.persist(
        async () => {
          const size = part.end - part.start;
          const signed = this.ok(await backend.partUrl(videoId, { number: part.number, size, last: part.last }));
          await this.deps.put(signed.part.url, await source.read(part.start, part.end));
        },
        () => this.show({ reason: "unreachable" }),
      );
      this.done = part.number;
      this.show({ sent: part.end, reason: undefined, phase: this.ended ? "finishing" : "uploading" });
    }
    this.show({ phase: "finishing" });
    const total = this.total ?? 0;
    await this.persist(
      async () => {
        this.ok(await backend.complete(videoId, total));
      },
      () => this.show({ reason: "unreachable" }),
    );
    this.show({ phase: "done", reason: undefined, sent: total });
  }
}
