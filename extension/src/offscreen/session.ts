// One recording: the streams it reads, the recorders, and the files they write.
//
// Each source is its own file: the screen (with the sound of the screen and of the microphone mixed into
// it) and, separately, the camera. They are not combined into one picture while recording, because a
// picture that has been flattened cannot be edited: the face cannot be moved out of the way of what is
// happening on the screen, or reframed for a vertical clip. The editor puts them together later.

import type { FailureCode, RecordingOptions, SavedFile, Track } from "../shared/messages";
import { fileName, pickMime } from "../shared/format";
import { DiskError, DiskWriter, readFile, writeJson } from "./opfs";

/** A failure with the word the rest of the extension understands it by. */
export class RecorderError extends Error {
  constructor(
    readonly code: FailureCode,
    message: string,
  ) {
    super(message);
  }
}

/** What the browser or the disk said, in words a person can act on. */
export function classify(error: unknown): RecorderError {
  if (error instanceof RecorderError) return error;
  const name = error instanceof DiskError ? error.kind : (error as { name?: string } | null)?.name;
  switch (name) {
    case "NotAllowedError":
    case "SecurityError":
      return new RecorderError("permission_denied", "The browser did not allow it. Allow the camera and microphone for Zeno and try again.");
    case "NotFoundError":
    case "OverconstrainedError":
    case "NotReadableError":
      return new RecorderError("device_not_found", "That camera or microphone could not be opened. Is another app using it?");
    case "QuotaExceededError":
      return new RecorderError("storage_full", "There is no room left to save the recording. What was recorded so far is kept.");
    default:
      return new RecorderError("unexpected", "Something went wrong while recording.");
  }
}

const SCREEN_BITS_PER_SECOND = 6_000_000;
const CAMERA_BITS_PER_SECOND = 2_500_000;
const AUDIO_BITS_PER_SECOND = 128_000;
/** How often a chunk is written and flushed to disk: the most a crash can cost. */
const TIMESLICE_MS = 1000;

interface Run {
  track: Track;
  file: string;
  mime: string;
  recorder: MediaRecorder;
  writer: DiskWriter;
  startedAt: number;
}

export interface Hooks {
  /** The person pressed the browser's own "Stop sharing". */
  onSourceEnded(): void;
  /** Something broke after recording had started (the disk, usually). */
  onFailure(error: RecorderError): void;
}

/**
 * The browser's own picker: the person chooses a tab, a window or the whole screen, and ticks "Share audio"
 * if they want its sound. Closing it without choosing is not a failure: it is a person changing their mind.
 */
async function pickScreen(withSound: boolean): Promise<MediaStream> {
  try {
    return await navigator.mediaDevices.getDisplayMedia({ video: { frameRate: 30 }, audio: withSound });
  } catch (error) {
    if ((error as { name?: string }).name === "NotAllowedError") throw new RecorderError("cancelled", "Nothing was chosen.");
    throw error;
  }
}

/** One audio track from several: the screen's own sound and the microphone, mixed. */
function mix(tracks: MediaStreamTrack[], cleanups: (() => void)[]): MediaStreamTrack | null {
  if (tracks.length === 0) return null;
  const [only] = tracks;
  if (tracks.length === 1 && only) return only;
  const context = new AudioContext();
  const destination = context.createMediaStreamDestination();
  for (const track of tracks) context.createMediaStreamSource(new MediaStream([track])).connect(destination);
  cleanups.push(() => void context.close());
  return destination.stream.getAudioTracks()[0] ?? null;
}

export class Session {
  private runs: Run[] = [];
  private cleanups: (() => void)[] = [];
  private failed = false;
  startedAt = 0;

  private constructor(
    readonly id: string,
    private readonly hooks: Hooks,
  ) {}

  /**
   * Opens the sources, the files and the recorders, and starts. If anything fails, everything opened so
   * far is closed again and the failure is thrown with its word.
   */
  static async start(id: string, options: RecordingOptions, hooks: Hooks): Promise<Session> {
    const session = new Session(id, hooks);
    try {
      await session.open(options);
      return session;
    } catch (error) {
      await session.abandon();
      throw classify(error);
    }
  }

  private async open(options: RecordingOptions): Promise<void> {
    const mime = pickMime((type) => MediaRecorder.isTypeSupported(type));
    if (!mime) throw new RecorderError("recorder_unsupported", "This browser cannot record video.");
    if (!options.screen && !options.camera) throw new RecorderError("nothing_to_record", "Choose the screen or the camera to record.");

    const take = (stream: MediaStream): MediaStream => {
      this.cleanups.push(() => stream.getTracks().forEach((track) => track.stop()));
      return stream;
    };
    const screen = options.screen ? take(await pickScreen(options.systemAudio)) : null;
    const microphone = options.microphone
      ? take(await navigator.mediaDevices.getUserMedia({ audio: { deviceId: options.microphoneId ? { exact: options.microphoneId } : undefined, echoCancellation: true, noiseSuppression: true, autoGainControl: true } }))
      : null;
    const camera = options.camera
      ? take(await navigator.mediaDevices.getUserMedia({ video: { deviceId: options.cameraId ? { exact: options.cameraId } : undefined, width: { ideal: 1280 }, height: { ideal: 720 }, frameRate: { ideal: 30 } } }))
      : null;

    const audio = mix([...(screen?.getAudioTracks() ?? []), ...(microphone?.getAudioTracks() ?? [])], this.cleanups);
    // With a screen, its file carries the sound. With only a camera, the camera's does.
    const streams: { track: Track; stream: MediaStream; bits: number }[] = [];
    if (screen) streams.push({ track: "screen", stream: new MediaStream([...screen.getVideoTracks(), ...(audio ? [audio] : [])]), bits: SCREEN_BITS_PER_SECOND });
    if (camera) streams.push({ track: "camera", stream: new MediaStream([...camera.getVideoTracks(), ...(!screen && audio ? [audio] : [])]), bits: CAMERA_BITS_PER_SECOND });

    // The person ending the share from the browser's own bar ends the recording, with what there is.
    screen?.getVideoTracks()[0]?.addEventListener("ended", () => this.hooks.onSourceEnded());

    const startedAt = Date.now();
    for (const { track, stream, bits } of streams) {
      const file = `${track}.webm`;
      const writer = await DiskWriter.open(["recordings", this.id, file]);
      const recorder = new MediaRecorder(stream, { mimeType: mime, videoBitsPerSecond: bits, audioBitsPerSecond: AUDIO_BITS_PER_SECOND });
      const run: Run = { track, file, mime, recorder, writer, startedAt: 0 };
      this.runs.push(run);
      recorder.ondataavailable = (event) => {
        if (event.data.size > 0) void run.writer.write(event.data).catch((error) => this.fail(error));
      };
      recorder.onerror = () => this.fail(new RecorderError("unexpected", "The recorder stopped unexpectedly."));
    }
    // Start them together, and note when each really began: the editor lines the tracks up by it.
    await Promise.all(
      this.runs.map(
        (run) =>
          new Promise<void>((resolve) => {
            run.recorder.onstart = () => {
              run.startedAt = Date.now();
              resolve();
            };
            run.recorder.start(TIMESLICE_MS);
          }),
      ),
    );
    this.startedAt = startedAt;
    await writeJson(["recordings", this.id, "manifest.json"], this.manifest("recording"));
  }

  private manifest(status: "recording" | "complete", extra: Record<string, unknown> = {}) {
    return {
      version: 1,
      id: this.id,
      status,
      startedAt: this.startedAt,
      tracks: this.runs.map((run) => ({ track: run.track, file: run.file, mime: run.mime, startedAt: run.startedAt })),
      ...extra,
    };
  }

  get tracks(): Track[] {
    return this.runs.map((run) => run.track);
  }

  private fail(error: unknown): void {
    if (this.failed) return;
    this.failed = true;
    this.hooks.onFailure(classify(error));
  }

  /** Stops the recorders, finishes the files, and hands back what was saved. */
  async stop(): Promise<{ durationMs: number; files: (SavedFile & { url: string })[] }> {
    await Promise.all(
      this.runs.map(
        (run) =>
          new Promise<void>((resolve) => {
            if (run.recorder.state === "inactive") return resolve();
            run.recorder.addEventListener("stop", () => resolve(), { once: true });
            run.recorder.stop();
          }),
      ),
    );
    const sizes: number[] = [];
    let problem: unknown = null;
    for (const run of this.runs) {
      try {
        sizes.push(await run.writer.close());
      } catch (error) {
        problem ??= error;
        sizes.push(0);
      }
    }
    this.release();
    const durationMs = Date.now() - this.startedAt;
    await writeJson(["recordings", this.id, "manifest.json"], this.manifest("complete", { endedAt: Date.now(), durationMs, bytes: sizes })).catch(() => undefined);
    if (problem) throw classify(problem);

    const files: (SavedFile & { url: string })[] = [];
    for (const [index, run] of this.runs.entries()) {
      const stored = await readFile(["recordings", this.id, run.file]);
      files.push({ track: run.track, name: fileName(run.track, this.startedAt), bytes: sizes[index] ?? stored.size, mime: run.mime, url: URL.createObjectURL(stored) });
    }
    return { durationMs, files };
  }

  /** Closes everything opened, after a failure to start. What was written stays on disk. */
  private async abandon(): Promise<void> {
    for (const run of this.runs) {
      if (run.recorder.state !== "inactive") run.recorder.stop();
      await run.writer.close().catch(() => undefined);
    }
    this.release();
  }

  private release(): void {
    for (const cleanup of this.cleanups.splice(0)) cleanup();
  }
}
