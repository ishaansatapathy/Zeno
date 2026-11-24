// What the parts of the extension say to each other.
//
// There are four: the popup (what the person sees), the service worker (who owns the state, and may be
// stopped by the browser at any moment), the offscreen document (the only place that can hold media
// streams and a recorder without a visible page) and a camera bubble drawn on the page being shared.
// Every message is typed here, once, so a part cannot say something the others do not understand.

/** What to record. Each is a separate track on disk: nothing is flattened into one picture. */
export interface RecordingOptions {
  /** The screen, a window or a tab, chosen in the browser's own picker. */
  screen: boolean;
  /** Sound the shared screen or tab is making, if the picker is given permission to offer it. */
  systemAudio: boolean;
  microphone: boolean;
  camera: boolean;
  microphoneId?: string;
  cameraId?: string;
  /** Count 3, 2, 1 once everything is chosen and open, before recording begins. */
  countdown: boolean;
}

export type Track = "screen" | "camera";

/** Why a saved recording says something about how it ended, when it was not the person pressing stop. */
export type SavedNote = "recovered" | "paused_too_long";

/** A recording that has been written to disk. */
export interface SavedFile {
  track: Track;
  name: string;
  bytes: number;
  mime: string;
}

export type FailureCode =
  | "permission_denied"
  | "device_not_found"
  | "nothing_to_record"
  /** The person closed the browser's picker without choosing. Not an error: nothing is shown. */
  | "cancelled"
  | "recorder_unsupported"
  | "storage_full"
  | "unexpected";

/** How the upload of a recording is going. The recording itself never depends on it: it is saved on disk either way. */
export interface UploadView {
  /**
   * `off`: nothing is uploaded (see `reason`). `connecting`: getting the upload and the link ready, or the
   * network is in the way. `uploading`: parts are going up while the recording goes on. `finishing`: the
   * recording has ended and the last of it is going up. `done`: it is all in Zeno. `failed`: it could not be.
   */
  phase: "off" | "connecting" | "uploading" | "finishing" | "done" | "failed";
  /** `signed_out`: no Zeno session. `unreachable`: Zeno cannot be reached right now (it keeps trying). */
  reason?: "signed_out" | "unreachable" | "refused" | undefined;
  /** Bytes of the recording that are in Zeno. */
  sent: number;
  /** The size of the whole recording, once it has ended. */
  total: number | null;
  /** The address to share, as soon as it exists (before the recording ends). */
  url: string | null;
  /** Whether the link has been put on the clipboard. */
  copied: boolean;
  message?: string | undefined;
}

/** A recording an earlier run left on this computer, which the person can finish. */
export interface Unfinished {
  id: string;
  startedAt: number;
  /** How long it ran, as far as is known. */
  durationMs: number;
  bytes: number;
  kind: "interrupted" | "upload" | "download";
}

/** Who is signed in to Zeno in this browser, as far as the extension can tell. */
export type Account = { status: "signed_in"; name: string; email: string } | { status: "signed_out" } | { status: "unreachable" };

// ---- popup -> service worker ----------------------------------------------------------------------

export type Command =
  | { type: "recorder.start"; options: RecordingOptions }
  | { type: "recorder.stop" }
  /** Holds the recording without ending it: nothing is written until it is resumed. */
  | { type: "recorder.pause" }
  | { type: "recorder.resume" }
  /** Changes one's mind during the count: nothing has been recorded. */
  | { type: "recorder.cancel" }
  | { type: "recorder.status" }
  /** The recordings an earlier run left unfinished (and a tidy-up of the finished ones that are old). */
  | { type: "recorder.unfinished" }
  /** Finishes one of them: closes it off if it was cut short, and uploads what Zeno does not have. */
  | { type: "recorder.recover"; id: string }
  /** Deletes one of them from this computer, for good. */
  | { type: "recorder.discard"; id: string }
  /** Asks who is signed in to Zeno. */
  | { type: "recorder.account" }
  /** Leaves the "saved" or "failed" screen and goes back to the start. */
  | { type: "recorder.dismiss" };

// ---- service worker -> offscreen document ---------------------------------------------------------

export type OffscreenCommand =
  | {
      target: "offscreen";
      type: "start";
      id: string;
      options: RecordingOptions;
    }
  | { target: "offscreen"; type: "stop"; id: string }
  | { target: "offscreen"; type: "pause"; id: string }
  | { target: "offscreen"; type: "resume"; id: string }
  /** Before the count reaches zero: forget it. */
  | { target: "offscreen"; type: "cancel"; id: string }
  | { target: "offscreen"; type: "recover"; id: string };

// ---- offscreen document -> service worker ---------------------------------------------------------

export type OffscreenEvent =
  | { target: "background"; type: "counting"; id: string; remaining: number }
  | { target: "background"; type: "started"; id: string; startedAt: number; tracks: Track[] }
  | { target: "background"; type: "paused"; id: string; at: number }
  | { target: "background"; type: "resumed"; id: string; at: number }
  | {
      target: "background";
      type: "stopped";
      id: string;
      durationMs: number;
      files: (SavedFile & { url: string })[];
      note?: SavedNote;
    }
  | { target: "background"; type: "failed"; id: string; code: FailureCode; message: string }
  | { target: "background"; type: "upload"; id: string; view: UploadView };

export function isOffscreenEvent(message: unknown): message is OffscreenEvent {
  return typeof message === "object" && message !== null && (message as { target?: unknown }).target === "background";
}

export function isOffscreenCommand(message: unknown): message is OffscreenCommand {
  return typeof message === "object" && message !== null && (message as { target?: unknown }).target === "offscreen";
}

export function isCommand(message: unknown): message is Command {
  const type = (message as { type?: unknown } | null)?.type;
  return typeof type === "string" && type.startsWith("recorder.") && !isOffscreenEvent(message) && !isOffscreenCommand(message);
}
