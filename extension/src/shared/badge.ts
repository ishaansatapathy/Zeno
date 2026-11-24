// What the toolbar icon says while the popup is closed: the one place a person can see that something is
// being recorded, paused or counted down without opening anything.

import { elapsedMs, type RecorderState } from "./state";

export interface Badge {
  /** Up to four characters fit. Empty clears it. */
  text: string;
  color: string;
}

const NONE: Badge = { text: "", color: "#000000" };

/** Muted, like the rest: a deep red for recording, greys for waiting. */
export function badgeFor(state: RecorderState): Badge {
  switch (state.phase) {
    case "counting":
      return state.remaining > 0 ? { text: String(state.remaining), color: "#3b3f46" } : NONE;
    case "recording":
      return state.pausedAt === null ? { text: "REC", color: "#8f2f2f" } : { text: "II", color: "#5b5f66" };
    case "recovering":
      return { text: "...", color: "#3b3f46" };
    case "failed":
      return { text: "!", color: "#8f2f2f" };
    default:
      return NONE;
  }
}

/** Not used for the badge itself: the time a screen reader is given for a recording, in words. */
export function spokenTime(state: Parameters<typeof elapsedMs>[0], now: number): string {
  const seconds = Math.floor(elapsedMs(state, now) / 1000);
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  if (minutes === 0) return `${rest} ${rest === 1 ? "second" : "seconds"}`;
  return `${minutes} ${minutes === 1 ? "minute" : "minutes"} ${rest} ${rest === 1 ? "second" : "seconds"}`;
}
