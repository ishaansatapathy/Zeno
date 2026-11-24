// Numbers that are fixed when the extension is built (see scripts/build.mjs), apart from the pure logic that is
// given them, so a test can try any value.

declare const __ZENO_MAX_PAUSE_MS__: number;

/** Seconds counted before recording begins. */
export const COUNTDOWN_SECONDS = 3;

/**
 * How long a recording may be left paused before it is stopped and saved. A paused recording still holds the
 * screen and the camera, so one that is forgotten would leave them shared; what was recorded is kept.
 */
export const MAX_PAUSE_MS: number = __ZENO_MAX_PAUSE_MS__;

/** After the count, the page is given this long to take the number off before the screen is recorded. */
export const AFTER_COUNT_MS = 250;
