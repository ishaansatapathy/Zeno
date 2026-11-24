// The recorder's life, as one pure function.
//
// The service worker may be stopped by the browser at any moment and started again for the next
// message, so nothing can live in its memory: the state is stored (chrome.storage.session) and every
// change goes through `reduce`. A message that arrives in a state where it makes no sense (a second
// "start", a "stopped" nobody asked for) changes nothing, so a repeated or late message cannot
// corrupt a recording.

import type { FailureCode, SavedFile, SavedNote, Track, UploadView } from "./messages";

export type RecorderState =
  | { phase: "idle" }
  /** The browser's own picker is open. */
  | { phase: "choosing"; id: string }
  | { phase: "starting"; id: string }
  /** Everything is open and the person is getting ready: nothing is recorded until the count reaches zero. */
  | { phase: "counting"; id: string; remaining: number }
  | { phase: "recording"; id: string; startedAt: number; tracks: Track[]; pausedMs: number; pausedAt: number | null; upload?: UploadView }
  | { phase: "stopping"; id: string; upload?: UploadView }
  /** An earlier recording is being finished: closed off if it was cut short, and uploaded if it was not. */
  | { phase: "recovering"; id: string; upload?: UploadView }
  | { phase: "saved"; id: string; durationMs: number; files: SavedFile[]; note?: SavedNote; upload?: UploadView }
  | { phase: "failed"; code: FailureCode; message: string };

/** The recording state while it is going: what says how long it has been. */
export type Running = Extract<RecorderState, { phase: "recording" }>;

export type RecorderEvent =
  /** `id` names the recording from this moment on. */
  | { type: "start-requested"; id: string; screen: boolean }
  | { type: "picker-cancelled" }
  | { type: "picked" }
  /** `remaining` seconds are left before recording begins. */
  | { type: "counting"; id: string; remaining: number }
  | { type: "started"; id: string; startedAt: number; tracks: Track[] }
  /** `at` is when the recorder really paused or resumed, not when the message got here. */
  | { type: "paused"; id: string; at: number }
  | { type: "resumed"; id: string; at: number }
  | { type: "stop-requested" }
  | { type: "stopped"; id: string; durationMs: number; files: SavedFile[]; note?: SavedNote }
  /** Finish the recording `id`, which an earlier run left on the disk. */
  | { type: "recover-requested"; id: string }
  | { type: "failed"; code: FailureCode; message: string }
  /** How the upload is going. It outlives the recording: it can finish after "stopped". */
  | { type: "upload"; id: string; view: UploadView }
  | { type: "dismissed" };

export const IDLE: RecorderState = { phase: "idle" };

/** `true` while there is a recording to look after: from the picker to the end of the save. */
export function isBusy(state: RecorderState): boolean {
  return (
    state.phase === "choosing" ||
    state.phase === "starting" ||
    state.phase === "counting" ||
    state.phase === "recording" ||
    state.phase === "stopping" ||
    state.phase === "recovering"
  );
}

/** How long has been recorded: the time since the start, less the time paused. Stands still while paused. */
export function elapsedMs(state: Pick<Running, "startedAt" | "pausedMs" | "pausedAt">, now: number): number {
  return Math.max(0, (state.pausedAt ?? now) - state.startedAt - state.pausedMs);
}

export function reduce(state: RecorderState, event: RecorderEvent): RecorderState {
  switch (event.type) {
    case "start-requested":
      // Only from rest. The picker comes first when the screen is wanted; otherwise straight to starting.
      if (state.phase !== "idle" && state.phase !== "saved" && state.phase !== "failed") return state;
      return event.screen ? { phase: "choosing", id: event.id } : { phase: "starting", id: event.id };
    case "picker-cancelled":
      // Also from the count: changing one's mind before it reaches zero records nothing.
      return state.phase === "choosing" || state.phase === "starting" || state.phase === "counting" ? IDLE : state;
    case "picked":
      return state.phase === "choosing" ? { phase: "starting", id: state.id } : state;
    case "counting":
      return (state.phase === "starting" || state.phase === "choosing" || state.phase === "counting") && state.id === event.id
        ? { phase: "counting", id: event.id, remaining: event.remaining }
        : state;
    case "started":
      // From choosing and counting as well: the picker belongs to the recorder, which says nothing until it has begun.
      return (state.phase === "starting" || state.phase === "choosing" || state.phase === "counting") && state.id === event.id
        ? { phase: "recording", id: event.id, startedAt: event.startedAt, tracks: event.tracks, pausedMs: 0, pausedAt: null }
        : state;
    case "paused":
      // A repeated word changes nothing: the pause that is already going on keeps the moment it began.
      return state.phase === "recording" && state.id === event.id && state.pausedAt === null ? { ...state, pausedAt: event.at } : state;
    case "resumed":
      return state.phase === "recording" && state.id === event.id && state.pausedAt !== null
        ? { ...state, pausedMs: state.pausedMs + Math.max(0, event.at - state.pausedAt), pausedAt: null }
        : state;
    case "stop-requested":
      return state.phase === "recording" ? { phase: "stopping", id: state.id, ...(state.upload ? { upload: state.upload } : {}) } : state;
    case "recover-requested":
      // Only from rest, like a start: one thing at a time. From "saved" too, to try an upload that did not finish again.
      return state.phase === "idle" || state.phase === "saved" || state.phase === "failed" ? { phase: "recovering", id: event.id } : state;
    case "stopped":
      // From stopping, and also from recording: the person can end it from the browser's own bar. From recovering:
      // that is how a recovered recording comes to be saved.
      if ((state.phase === "stopping" || state.phase === "recording" || state.phase === "recovering") && state.id === event.id) {
        return {
          phase: "saved",
          id: event.id,
          durationMs: event.durationMs,
          files: event.files,
          ...(event.note ? { note: event.note } : {}),
          ...(state.upload ? { upload: state.upload } : {}),
        };
      }
      return state;
    case "failed":
      // A failure ends whatever was going on, and is shown until the person dismisses it. It does not
      // overwrite a recording that was already saved.
      return state.phase === "saved" ? state : { phase: "failed", code: event.code, message: event.message };
    case "upload":
      // Only for the recording it belongs to, and only while there is one to show it on.
      if ((state.phase === "recording" || state.phase === "stopping" || state.phase === "recovering" || state.phase === "saved") && state.id === event.id) {
        return { ...state, upload: event.view };
      }
      return state;
    case "dismissed":
      return state.phase === "saved" || state.phase === "failed" ? IDLE : state;
  }
}
