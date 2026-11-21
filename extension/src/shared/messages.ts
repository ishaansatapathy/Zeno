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
}

export type Track = "screen" | "camera";

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

// ---- popup -> service worker ----------------------------------------------------------------------

export type Command =
  | { type: "recorder.start"; options: RecordingOptions }
  | { type: "recorder.stop" }
  | { type: "recorder.status" }
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
  | { target: "offscreen"; type: "stop"; id: string };

// ---- offscreen document -> service worker ---------------------------------------------------------

export type OffscreenEvent =
  | { target: "background"; type: "started"; id: string; startedAt: number; tracks: Track[] }
  | { target: "background"; type: "stopped"; id: string; durationMs: number; files: (SavedFile & { url: string })[] }
  | { target: "background"; type: "failed"; id: string; code: FailureCode; message: string };

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
