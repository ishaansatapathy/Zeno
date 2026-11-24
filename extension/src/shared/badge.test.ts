import { describe, expect, it } from "vitest";

import { badgeFor, spokenTime } from "./badge";
import { IDLE, type RecorderState } from "./state";

describe("the toolbar badge", () => {
  it("is empty at rest, while choosing and while saving", () => {
    expect(badgeFor(IDLE).text).toBe("");
    expect(badgeFor({ phase: "choosing", id: "a" }).text).toBe("");
    expect(badgeFor({ phase: "stopping", id: "a" }).text).toBe("");
    expect(badgeFor({ phase: "saved", id: "a", durationMs: 1, files: [] }).text).toBe("");
  });

  it("counts down, and clears when the count is over", () => {
    expect(badgeFor({ phase: "counting", id: "a", remaining: 3 }).text).toBe("3");
    expect(badgeFor({ phase: "counting", id: "a", remaining: 1 }).text).toBe("1");
    expect(badgeFor({ phase: "counting", id: "a", remaining: 0 }).text).toBe("");
  });

  it("says REC while recording and II while paused, in different colours", () => {
    const recording: RecorderState = { phase: "recording", id: "a", startedAt: 1, tracks: [], pausedMs: 0, pausedAt: null };
    const paused: RecorderState = { ...recording, pausedAt: 5 };
    expect(badgeFor(recording).text).toBe("REC");
    expect(badgeFor(paused).text).toBe("II");
    expect(badgeFor(recording).color).not.toBe(badgeFor(paused).color);
  });

  it("marks a failure, and a recovery that is going on", () => {
    expect(badgeFor({ phase: "failed", code: "unexpected", message: "x" }).text).toBe("!");
    expect(badgeFor({ phase: "recovering", id: "a" }).text).toBe("...");
  });

  it("never needs more than the four characters a badge has", () => {
    const states: RecorderState[] = [
      { phase: "counting", id: "a", remaining: 3 },
      { phase: "recording", id: "a", startedAt: 1, tracks: [], pausedMs: 0, pausedAt: null },
      { phase: "recording", id: "a", startedAt: 1, tracks: [], pausedMs: 0, pausedAt: 4 },
      { phase: "recovering", id: "a" },
      { phase: "failed", code: "unexpected", message: "x" },
    ];
    for (const state of states) expect(badgeFor(state).text.length).toBeLessThanOrEqual(4);
  });
});

describe("the time in words", () => {
  const base = { startedAt: 0, pausedMs: 0, pausedAt: null };
  it("says seconds, then minutes and seconds, with the right plural", () => {
    expect(spokenTime(base, 1000)).toBe("1 second");
    expect(spokenTime(base, 5000)).toBe("5 seconds");
    expect(spokenTime(base, 61_000)).toBe("1 minute 1 second");
    expect(spokenTime(base, 125_000)).toBe("2 minutes 5 seconds");
  });

  it("stands still while paused", () => {
    expect(spokenTime({ startedAt: 0, pausedMs: 0, pausedAt: 4000 }, 99_000)).toBe("4 seconds");
  });
});
