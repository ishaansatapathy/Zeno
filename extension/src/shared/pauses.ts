// Time spent paused, as plain arithmetic.
//
// A paused recording writes nothing: the file has no gap, it just continues, so its length is the time
// spent recording and not the time since the start. Everything that is shown or stored as "how long" goes
// through here, so the clock in the popup, the length of a saved recording and the length the editor will
// read from the manifest cannot disagree.

/** One pause, in wall-clock milliseconds. `to` is `null` while it is still going on. */
export interface Pause {
  from: number;
  to: number | null;
}

/** The part of a pause that lies inside `[start, end]`, in milliseconds. */
function within(pause: Pause, start: number, end: number): number {
  const from = Math.max(pause.from, start);
  const to = Math.min(pause.to ?? end, end);
  return Math.max(0, to - from);
}

/** How long was spent paused between `start` and `end`. A pause that has not ended is counted up to `end`. */
export function pausedBetween(pauses: readonly Pause[], start: number, end: number): number {
  return pauses.reduce((total, pause) => total + within(pause, start, end), 0);
}

/** How long was spent recording between `startedAt` and `endedAt`: the time that passed, less the time paused. */
export function activeMs(startedAt: number, endedAt: number, pauses: readonly Pause[]): number {
  return Math.max(0, endedAt - startedAt - pausedBetween(pauses, startedAt, endedAt));
}

/** Where, in the file, the recording was paused: the editor joins the tracks and finds the seams by these. */
export interface Joint {
  /** Milliseconds into the file at which the pause happened. */
  at: number;
  /** How long it lasted, which is not in the file at all. */
  ms: number;
}

/** The pauses of a recording, in the file's own time. A pause that is still going on ends at `now`. */
export function joints(startedAt: number, pauses: readonly Pause[], now: number): Joint[] {
  const marks: Joint[] = [];
  let before = 0;
  for (const pause of [...pauses].sort((a, b) => a.from - b.from)) {
    const ms = within(pause, startedAt, now);
    if (ms === 0) continue;
    marks.push({ at: Math.max(0, pause.from - startedAt - before), ms });
    before += ms;
  }
  return marks;
}

/** Whether a recording has been paused for longer than it may be left (so the screen is not left shared by accident). */
export function pausedTooLong(pauses: readonly Pause[], now: number, limitMs: number): boolean {
  const open = pauses.find((pause) => pause.to === null);
  return open !== undefined && now - open.from >= limitMs;
}
