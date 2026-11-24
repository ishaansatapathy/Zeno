import { describe, expect, it } from "vitest";

import type { SavedFile, UploadView } from "./messages";
import { elapsedMs, IDLE, isBusy, reduce, type RecorderEvent, type RecorderState } from "./state";

const file: SavedFile = { track: "screen", name: "screen.webm", bytes: 1234, mime: "video/webm" };

function run(events: RecorderEvent[], from: RecorderState = IDLE): RecorderState {
  return events.reduce(reduce, from);
}

describe("a recording from start to finish", () => {
  it("goes through the picker when the screen is wanted", () => {
    const state = run([
      { type: "start-requested", id: "r1", screen: true },
      { type: "picked" },
      { type: "started", id: "r1", startedAt: 1000, tracks: ["screen", "camera"] },
      { type: "stop-requested" },
      { type: "stopped", id: "r1", durationMs: 5000, files: [file] },
    ]);
    expect(state).toEqual({ phase: "saved", id: "r1", durationMs: 5000, files: [file] });
  });

  it("goes straight to starting when there is no screen to pick", () => {
    expect(run([{ type: "start-requested", id: "r2", screen: false }])).toEqual({ phase: "starting", id: "r2" });
  });

  it("is busy from the picker until it is saved, and not before or after", () => {
    expect(isBusy(IDLE)).toBe(false);
    expect(isBusy({ phase: "choosing", id: "a" })).toBe(true);
    expect(isBusy({ phase: "starting", id: "a" })).toBe(true);
    expect(isBusy({ phase: "recording", id: "a", startedAt: 1, tracks: [], pausedMs: 0, pausedAt: null })).toBe(true);
    expect(isBusy({ phase: "stopping", id: "a" })).toBe(true);
    expect(isBusy({ phase: "saved", id: "a", durationMs: 1, files: [] })).toBe(false);
  });
});

describe("the browser's picker belongs to the recorder", () => {
  it("starts recording straight from choosing, because the recorder says nothing until it has begun", () => {
    const state = run([
      { type: "start-requested", id: "r1", screen: true },
      { type: "started", id: "r1", startedAt: 5, tracks: ["screen"] },
    ]);
    expect(state).toEqual({ phase: "recording", id: "r1", startedAt: 5, tracks: ["screen"], pausedMs: 0, pausedAt: null });
  });

  it("goes back to rest when the picker is closed without choosing, from either waiting state", () => {
    expect(run([{ type: "start-requested", id: "r1", screen: true }, { type: "picker-cancelled" }])).toEqual(IDLE);
    expect(run([{ type: "start-requested", id: "r1", screen: false }, { type: "picker-cancelled" }])).toEqual(IDLE);
  });

  it("does not let a late cancel end a recording that has begun", () => {
    const recording: RecorderState = { phase: "recording", id: "r1", startedAt: 1, tracks: ["screen"], pausedMs: 0, pausedAt: null };
    expect(reduce(recording, { type: "picker-cancelled" })).toBe(recording);
  });
});

describe("a person who changes their mind", () => {
  it("can close the picker without recording anything", () => {
    expect(run([{ type: "start-requested", id: "r1", screen: true }, { type: "picker-cancelled" }])).toEqual(IDLE);
  });

  it("can end a recording from the browser's own Stop sharing bar", () => {
    const state = run([
      { type: "start-requested", id: "r1", screen: true },
      { type: "picked" },
      { type: "started", id: "r1", startedAt: 1, tracks: ["screen"] },
      { type: "stopped", id: "r1", durationMs: 800, files: [file] },
    ]);
    expect(state.phase).toBe("saved");
  });
});

describe("messages that make no sense are ignored", () => {
  const recording: RecorderState = { phase: "recording", id: "r1", startedAt: 1, tracks: ["screen"], pausedMs: 0, pausedAt: null };

  it("a second start while recording changes nothing", () => {
    expect(reduce(recording, { type: "start-requested", id: "r2", screen: true })).toBe(recording);
  });

  it("a start, stop or finish for another recording changes nothing", () => {
    expect(reduce({ phase: "starting", id: "r1" }, { type: "started", id: "other", startedAt: 1, tracks: [] })).toEqual({ phase: "starting", id: "r1" });
    expect(reduce(recording, { type: "stopped", id: "other", durationMs: 1, files: [] })).toBe(recording);
  });

  it("a stop with nothing recording, and a late finish, change nothing", () => {
    expect(reduce(IDLE, { type: "stop-requested" })).toBe(IDLE);
    expect(reduce(IDLE, { type: "stopped", id: "r1", durationMs: 1, files: [] })).toBe(IDLE);
  });

  it("the picker's answer after it was closed changes nothing", () => {
    expect(reduce(IDLE, { type: "picked" })).toBe(IDLE);
  });

  it("cannot be dismissed while it is busy", () => {
    expect(reduce(recording, { type: "dismissed" })).toBe(recording);
  });
});

describe("failures", () => {
  it("end whatever was going on and stay until dismissed", () => {
    const failed = run([{ type: "start-requested", id: "r1", screen: false }, { type: "failed", code: "permission_denied", message: "No camera." }]);
    expect(failed).toEqual({ phase: "failed", code: "permission_denied", message: "No camera." });
    expect(reduce(failed, { type: "dismissed" })).toEqual(IDLE);
  });

  it("do not overwrite a recording that was already saved", () => {
    const saved: RecorderState = { phase: "saved", id: "r1", durationMs: 1, files: [file] };
    expect(reduce(saved, { type: "failed", code: "unexpected", message: "late" })).toBe(saved);
  });

  it("allow another try from the failed screen", () => {
    const failed: RecorderState = { phase: "failed", code: "unexpected", message: "x" };
    expect(reduce(failed, { type: "start-requested", id: "r3", screen: false })).toEqual({ phase: "starting", id: "r3" });
  });
});

describe("the upload, which outlives the recording", () => {
  const view: UploadView = { phase: "uploading", sent: 100, total: null, url: "https://zeno.test/s/abc", copied: false };
  const recording: RecorderState = { phase: "recording", id: "r1", startedAt: 1, tracks: ["screen"], pausedMs: 0, pausedAt: null };

  it("is shown on the recording it belongs to", () => {
    expect(run([{ type: "upload", id: "r1", view }], recording)).toEqual({ ...recording, upload: view });
  });

  it("follows the recording through stopping and saved, and goes on changing after it", () => {
    const state = run(
      [
        { type: "upload", id: "r1", view },
        { type: "stop-requested" },
        { type: "stopped", id: "r1", durationMs: 5000, files: [file] },
        { type: "upload", id: "r1", view: { ...view, phase: "done", sent: 500, total: 500, copied: true } },
      ],
      recording,
    );
    expect(state).toMatchObject({ phase: "saved", id: "r1", upload: { phase: "done", copied: true, url: "https://zeno.test/s/abc" } });
  });

  it("ignores a late word about another recording, or about nothing", () => {
    expect(run([{ type: "upload", id: "old", view }], recording)).toEqual(recording);
    expect(run([{ type: "upload", id: "r1", view }])).toEqual(IDLE);
    expect(run([{ type: "upload", id: "r1", view }], { phase: "failed", code: "unexpected", message: "x" })).toEqual({ phase: "failed", code: "unexpected", message: "x" });
  });

  it("is gone when the person starts again", () => {
    const saved = run([{ type: "upload", id: "r1", view }, { type: "stopped", id: "r1", durationMs: 1, files: [file] }], recording);
    expect(run([{ type: "dismissed" }, { type: "start-requested", id: "r2", screen: false }], saved)).toEqual({ phase: "starting", id: "r2" });
  });
});

describe("counting down before it begins", () => {
  const counting: RecorderState = { phase: "counting", id: "r1", remaining: 3 };

  it("comes after the picker and before recording, and is busy", () => {
    const state = run([
      { type: "start-requested", id: "r1", screen: true },
      { type: "counting", id: "r1", remaining: 3 },
      { type: "counting", id: "r1", remaining: 2 },
    ]);
    expect(state).toEqual({ phase: "counting", id: "r1", remaining: 2 });
    expect(isBusy(counting)).toBe(true);
  });

  it("begins recording when the count is over, from the count and not before it", () => {
    expect(reduce(counting, { type: "started", id: "r1", startedAt: 9, tracks: ["screen"] })).toEqual({
      phase: "recording",
      id: "r1",
      startedAt: 9,
      tracks: ["screen"],
      pausedMs: 0,
      pausedAt: null,
    });
  });

  it("can be given up on, and nothing is recorded", () => {
    expect(reduce(counting, { type: "picker-cancelled" })).toEqual(IDLE);
  });

  it("is not stopped, paused or restarted by messages meant for a recording", () => {
    expect(reduce(counting, { type: "stop-requested" })).toBe(counting);
    expect(reduce(counting, { type: "paused", id: "r1", at: 5 })).toBe(counting);
    expect(reduce(counting, { type: "start-requested", id: "r2", screen: false })).toBe(counting);
  });

  it("ignores a count for another recording, or one that came after the recording began", () => {
    expect(reduce(counting, { type: "counting", id: "other", remaining: 1 })).toBe(counting);
    const recording: RecorderState = { phase: "recording", id: "r1", startedAt: 1, tracks: ["screen"], pausedMs: 0, pausedAt: null };
    expect(reduce(recording, { type: "counting", id: "r1", remaining: 1 })).toBe(recording);
    expect(reduce(IDLE, { type: "counting", id: "r1", remaining: 1 })).toBe(IDLE);
  });

  it("ends in a failure like any other waiting state", () => {
    expect(reduce(counting, { type: "failed", code: "unexpected", message: "x" })).toEqual({ phase: "failed", code: "unexpected", message: "x" });
  });
});

describe("pausing and resuming", () => {
  const recording: RecorderState = { phase: "recording", id: "r1", startedAt: 1000, tracks: ["screen"], pausedMs: 0, pausedAt: null };

  it("remembers when it paused, and how long it was paused once it resumes", () => {
    const paused = reduce(recording, { type: "paused", id: "r1", at: 4000 });
    expect(paused).toMatchObject({ phase: "recording", pausedAt: 4000, pausedMs: 0 });
    const resumed = reduce(paused, { type: "resumed", id: "r1", at: 9000 });
    expect(resumed).toMatchObject({ pausedAt: null, pausedMs: 5000 });
    const again = run([{ type: "paused", id: "r1", at: 12_000 }, { type: "resumed", id: "r1", at: 13_000 }], resumed);
    expect(again).toMatchObject({ pausedAt: null, pausedMs: 6000 });
  });

  it("keeps the moment of the first pause when it is told twice, and ignores a resume that was not paused", () => {
    const paused = reduce(recording, { type: "paused", id: "r1", at: 4000 });
    expect(reduce(paused, { type: "paused", id: "r1", at: 7000 })).toBe(paused);
    expect(reduce(recording, { type: "resumed", id: "r1", at: 7000 })).toBe(recording);
  });

  it("ignores a pause or a resume for another recording, or when nothing is being recorded", () => {
    expect(reduce(recording, { type: "paused", id: "other", at: 4000 })).toBe(recording);
    expect(reduce(IDLE, { type: "paused", id: "r1", at: 4000 })).toBe(IDLE);
    expect(reduce({ phase: "stopping", id: "r1" }, { type: "paused", id: "r1", at: 4000 })).toEqual({ phase: "stopping", id: "r1" });
    expect(reduce({ phase: "stopping", id: "r1" }, { type: "resumed", id: "r1", at: 4000 })).toEqual({ phase: "stopping", id: "r1" });
  });

  it("can be stopped while paused, and the upload carries on being shown", () => {
    const view: UploadView = { phase: "uploading", sent: 1, total: null, url: null, copied: false };
    const state = run(
      [
        { type: "upload", id: "r1", view },
        { type: "paused", id: "r1", at: 4000 },
        { type: "stop-requested" },
        { type: "stopped", id: "r1", durationMs: 3000, files: [file] },
      ],
      recording,
    );
    expect(state).toMatchObject({ phase: "saved", durationMs: 3000, upload: view });
  });

  it("never lets a clock that went backwards make a pause negative", () => {
    const paused = reduce(recording, { type: "paused", id: "r1", at: 4000 });
    expect(reduce(paused, { type: "resumed", id: "r1", at: 3000 })).toMatchObject({ pausedMs: 0 });
  });
});

describe("how long it has been recording", () => {
  it("is the time since the start, less the time paused, and stands still while paused", () => {
    expect(elapsedMs({ startedAt: 1000, pausedMs: 0, pausedAt: null }, 6000)).toBe(5000);
    expect(elapsedMs({ startedAt: 1000, pausedMs: 2000, pausedAt: null }, 6000)).toBe(3000);
    expect(elapsedMs({ startedAt: 1000, pausedMs: 2000, pausedAt: 5000 }, 99_000)).toBe(2000);
    expect(elapsedMs({ startedAt: 1000, pausedMs: 0, pausedAt: null }, 500)).toBe(0);
  });
});

describe("finishing a recording an earlier run left", () => {
  it("is allowed from rest, from a failure, and from a saved recording whose upload did not finish", () => {
    expect(reduce(IDLE, { type: "recover-requested", id: "old" })).toEqual({ phase: "recovering", id: "old" });
    expect(reduce({ phase: "failed", code: "unexpected", message: "x" }, { type: "recover-requested", id: "old" })).toEqual({ phase: "recovering", id: "old" });
    expect(reduce({ phase: "saved", id: "r1", durationMs: 1, files: [file] }, { type: "recover-requested", id: "r1" })).toEqual({ phase: "recovering", id: "r1" });
  });

  it("is refused while something else is going on, so there is one recording at a time", () => {
    const recording: RecorderState = { phase: "recording", id: "r1", startedAt: 1, tracks: ["screen"], pausedMs: 0, pausedAt: null };
    expect(reduce(recording, { type: "recover-requested", id: "old" })).toBe(recording);
    expect(reduce({ phase: "starting", id: "r1" }, { type: "recover-requested", id: "old" })).toEqual({ phase: "starting", id: "r1" });
    const recovering: RecorderState = { phase: "recovering", id: "old" };
    expect(reduce(recovering, { type: "recover-requested", id: "other" })).toBe(recovering);
    expect(isBusy(recovering)).toBe(true);
  });

  it("ends as a saved recording that says it was recovered, with the upload it made", () => {
    const view: UploadView = { phase: "done", sent: 5, total: 5, url: "https://zeno.test/s/abc", copied: true };
    const state = run(
      [
        { type: "recover-requested", id: "old" },
        { type: "upload", id: "old", view },
        { type: "stopped", id: "old", durationMs: 7000, files: [file], note: "recovered" },
      ],
    );
    expect(state).toEqual({ phase: "saved", id: "old", durationMs: 7000, files: [file], note: "recovered", upload: view });
  });

  it("ignores the end of a recording that is not the one being recovered, and cannot be stopped", () => {
    const recovering: RecorderState = { phase: "recovering", id: "old" };
    expect(reduce(recovering, { type: "stopped", id: "other", durationMs: 1, files: [] })).toBe(recovering);
    expect(reduce(recovering, { type: "stop-requested" })).toBe(recovering);
    expect(reduce(recovering, { type: "failed", code: "unexpected", message: "gone" })).toEqual({ phase: "failed", code: "unexpected", message: "gone" });
  });
});
