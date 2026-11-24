import { describe, expect, it } from "vitest";

import { activeMs, joints, pausedBetween, pausedTooLong, type Pause } from "./pauses";

describe("how long was spent recording", () => {
  it("is the whole time when there was no pause", () => {
    expect(activeMs(1000, 11_000, [])).toBe(10_000);
  });

  it("leaves out each pause", () => {
    const pauses: Pause[] = [
      { from: 3000, to: 5000 },
      { from: 8000, to: 9000 },
    ];
    expect(activeMs(1000, 11_000, pauses)).toBe(7000);
  });

  it("counts a pause that has not ended up to the moment asked about", () => {
    expect(activeMs(0, 10_000, [{ from: 4000, to: null }])).toBe(4000);
  });

  it("does not count a pause that began before the start or ends after the end", () => {
    expect(pausedBetween([{ from: 0, to: 5000 }], 2000, 10_000)).toBe(3000);
    expect(pausedBetween([{ from: 8000, to: 20_000 }], 2000, 10_000)).toBe(2000);
    expect(pausedBetween([{ from: 20_000, to: 30_000 }], 2000, 10_000)).toBe(0);
  });

  it("is never negative", () => {
    expect(activeMs(5000, 1000, [])).toBe(0);
    expect(activeMs(0, 1000, [{ from: 0, to: 5000 }])).toBe(0);
  });
});

describe("where the seams are in the file", () => {
  it("puts each pause where the file was when it happened, not where the clock was", () => {
    // Started at 1000. Recorded 2 s, paused 3 s, recorded 4 s, paused 1 s, recorded on.
    const pauses: Pause[] = [
      { from: 3000, to: 6000 },
      { from: 10_000, to: 11_000 },
    ];
    expect(joints(1000, pauses, 20_000)).toEqual([
      { at: 2000, ms: 3000 },
      { at: 6000, ms: 1000 },
    ]);
  });

  it("closes a pause that is still going on at the moment given, and ignores one that never lasted", () => {
    expect(joints(0, [{ from: 5000, to: null }], 7000)).toEqual([{ at: 5000, ms: 2000 }]);
    expect(joints(0, [{ from: 5000, to: 5000 }], 7000)).toEqual([]);
  });

  it("does not depend on the order the pauses were written in", () => {
    const pauses: Pause[] = [
      { from: 10_000, to: 11_000 },
      { from: 3000, to: 6000 },
    ];
    expect(joints(1000, pauses, 20_000).map((joint) => joint.at)).toEqual([2000, 6000]);
  });
});

describe("a recording left paused", () => {
  it("is told to stop once it has been paused for the limit, and not before", () => {
    const pauses: Pause[] = [{ from: 1000, to: null }];
    expect(pausedTooLong(pauses, 1000 + 29_999, 30_000)).toBe(false);
    expect(pausedTooLong(pauses, 1000 + 30_000, 30_000)).toBe(true);
  });

  it("is not told so when it is recording, or when its pauses were short", () => {
    expect(pausedTooLong([], 1e9, 30_000)).toBe(false);
    expect(pausedTooLong([{ from: 0, to: 100_000 }], 1e9, 30_000)).toBe(false);
  });
});
