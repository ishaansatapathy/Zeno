// What the share page shows, as plain functions: no React, no network, so each rule can be tested alone.

import type { ApiFailure, Schemas } from "@/lib/api/client";

export type ShareInfo = Schemas["ShareView"];

/** What a visitor is looking at. */
export type View =
  /** The link is good and its video can be played. */
  | { kind: "ready"; title: string; durationSeconds: number | null }
  /** The link is good and its video is not ready yet. */
  | { kind: "preparing"; title: string; status: "capturing" | "uploading" | "processing" }
  /** The video could not be processed. */
  | { kind: "failed"; title: string }
  /** The link needs a password, and until it is given nothing else is known. */
  | { kind: "protected" }
  /** The link is good, and has closed. */
  | { kind: "closed" };

/** What the page says a link is. A password link reveals nothing but that it needs one. */
export function viewOf(share: ShareInfo): View {
  if (share.requires_password) return { kind: "protected" };
  const title = share.title ?? "Untitled recording";
  switch (share.status) {
    case "ready":
      return { kind: "ready", title, durationSeconds: share.duration_seconds ?? null };
    case "failed":
      return { kind: "failed", title };
    case "capturing":
    case "uploading":
    case "processing":
      return { kind: "preparing", title, status: share.status };
    default:
      // A status this app has not heard of: say it is being prepared rather than offer a player
      // that cannot play it.
      return { kind: "preparing", title, status: "processing" };
  }
}

/** Seconds to wait before asking again whether a video is ready: 6, 9, 13, ... up to 30. */
export function pollDelaySeconds(attempt: number): number {
  return Math.min(30, Math.round(6 * 1.5 ** Math.max(0, attempt)));
}

/** The words for a video that is not ready. */
export function preparingText(status: "capturing" | "uploading" | "processing"): { heading: string; line: string } {
  switch (status) {
    case "capturing":
      return { heading: "Still being recorded", line: "This recording isn't finished yet. This page will update by itself when it is." };
    case "uploading":
      return { heading: "Still uploading", line: "The video is on its way. This page will update by itself when it is ready to watch." };
    case "processing":
      return { heading: "Getting it ready", line: "The video is being prepared for playing. This page will update by itself when it is ready." };
  }
}

// ---- pressing play ------------------------------------------------------------------------------

export type PlayerState =
  /** Nothing has been asked yet. `notice` explains why we are back here, if we are. */
  | { phase: "idle"; notice: string | null; passwordProblem: string | null }
  | { phase: "starting" }
  | { phase: "playing"; playlistUrl: string; expiresAt: number; watermark: string }
  /** The link closed while the visitor was getting here. */
  | { phase: "closed" }
  /** The video is not ready yet; the page should show that instead of a player. */
  | { phase: "preparing" };

export type PlayerEvent =
  | { type: "press" }
  | { type: "started"; playlistUrl: string; expiresAt: string; watermark: string }
  | { type: "refused"; failure: ApiFailure; retryWording: (failure: ApiFailure) => string }
  | { type: "ended"; notice: string };

const IDLE: PlayerState = { phase: "idle", notice: null, passwordProblem: null };

/** One transition of the player. Pure, so every way pressing play can end is a test. */
export function nextPlayerState(state: PlayerState, event: PlayerEvent): PlayerState {
  switch (event.type) {
    case "press":
      return state.phase === "starting" ? state : { phase: "starting" };
    case "started":
      return { phase: "playing", playlistUrl: event.playlistUrl, expiresAt: Date.parse(event.expiresAt), watermark: event.watermark };
    case "ended":
      return { phase: "idle", notice: event.notice, passwordProblem: null };
    case "refused": {
      const { failure } = event;
      switch (failure.code) {
        case "password_required":
          return { phase: "idle", notice: null, passwordProblem: "Enter the password to watch." };
        case "wrong_password":
          return { phase: "idle", notice: null, passwordProblem: "That password isn't right." };
        case "not_ready":
          return { phase: "preparing" };
        case "link_unavailable":
        case "not_found":
          return { phase: "closed" };
        default:
          return { phase: "idle", notice: event.retryWording(failure), passwordProblem: null };
      }
    }
  }
}

export const initialPlayerState = IDLE;
