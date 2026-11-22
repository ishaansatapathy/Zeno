// The recorder's life, as one pure function.
//
// The service worker may be stopped by the browser at any moment and started again for the next
// message, so nothing can live in its memory: the state is stored (chrome.storage.session) and every
// change goes through `reduce`. A message that arrives in a state where it makes no sense (a second
// "start", a "stopped" nobody asked for) changes nothing, so a repeated or late message cannot
// corrupt a recording.

import type { FailureCode, SavedFile, Track, UploadView } from "./messages";

export type RecorderState =
  | { phase: "idle" }
  /** The browser's own picker is open. */
  | { phase: "choosing"; id: string }
  | { phase: "starting"; id: string }
  | { phase: "recording"; id: string; startedAt: number; tracks: Track[]; upload?: UploadView }
  | { phase: "stopping"; id: string; upload?: UploadView }
  | { phase: "saved"; id: string; durationMs: number; files: SavedFile[]; upload?: UploadView }
  | { phase: "failed"; code: FailureCode; message: string };

export type RecorderEvent =
  /** `id` names the recording from this moment on. */
  | { type: "start-requested"; id: string; screen: boolean }
  | { type: "picker-cancelled" }
  | { type: "picked" }
  | { type: "started"; id: string; startedAt: number; tracks: Track[] }
  | { type: "stop-requested" }
  | { type: "stopped"; id: string; durationMs: number; files: SavedFile[] }
  | { type: "failed"; code: FailureCode; message: string }
  /** How the upload is going. It outlives the recording: it can finish after "stopped". */
  | { type: "upload"; id: string; view: UploadView }
  | { type: "dismissed" };

export const IDLE: RecorderState = { phase: "idle" };

/** `true` while there is a recording to look after: from the picker to the end of the save. */
export function isBusy(state: RecorderState): boolean {
  return state.phase === "choosing" || state.phase === "starting" || state.phase === "recording" || state.phase === "stopping";
}

export function reduce(state: RecorderState, event: RecorderEvent): RecorderState {
  switch (event.type) {
    case "start-requested":
      // Only from rest. The picker comes first when the screen is wanted; otherwise straight to starting.
      if (state.phase !== "idle" && state.phase !== "saved" && state.phase !== "failed") return state;
      return event.screen ? { phase: "choosing", id: event.id } : { phase: "starting", id: event.id };
    case "picker-cancelled":
      return state.phase === "choosing" || state.phase === "starting" ? IDLE : state;
    case "picked":
      return state.phase === "choosing" ? { phase: "starting", id: state.id } : state;
    case "started":
      // From choosing as well: the picker belongs to the recorder, which says nothing until it has begun.
      return (state.phase === "starting" || state.phase === "choosing") && state.id === event.id ? { phase: "recording", id: event.id, startedAt: event.startedAt, tracks: event.tracks } : state;
    case "stop-requested":
      return state.phase === "recording" ? { phase: "stopping", id: state.id, ...(state.upload ? { upload: state.upload } : {}) } : state;
    case "stopped":
      // From stopping, and also from recording: the person can end it from the browser's own bar.
      if ((state.phase === "stopping" || state.phase === "recording") && state.id === event.id) {
        return { phase: "saved", id: event.id, durationMs: event.durationMs, files: event.files, ...(state.upload ? { upload: state.upload } : {}) };
      }
      return state;
    case "failed":
      // A failure ends whatever was going on, and is shown until the person dismisses it. It does not
      // overwrite a recording that was already saved.
      return state.phase === "saved" ? state : { phase: "failed", code: event.code, message: event.message };
    case "upload":
      // Only for the recording it belongs to, and only while there is one to show it on.
      if ((state.phase === "recording" || state.phase === "stopping" || state.phase === "saved") && state.id === event.id) return { ...state, upload: event.view };
      return state;
    case "dismissed":
      return state.phase === "saved" || state.phase === "failed" ? IDLE : state;
  }
}
