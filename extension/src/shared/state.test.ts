import { describe, expect, it } from "vitest";

import type { SavedFile } from "./messages";
import { IDLE, isBusy, reduce, type RecorderEvent, type RecorderState } from "./state";

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
    expect(isBusy({ phase: "recording", id: "a", startedAt: 1, tracks: [] })).toBe(true);
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
    expect(state).toEqual({ phase: "recording", id: "r1", startedAt: 5, tracks: ["screen"] });
  });

  it("goes back to rest when the picker is closed without choosing, from either waiting state", () => {
    expect(run([{ type: "start-requested", id: "r1", screen: true }, { type: "picker-cancelled" }])).toEqual(IDLE);
    expect(run([{ type: "start-requested", id: "r1", screen: false }, { type: "picker-cancelled" }])).toEqual(IDLE);
  });

  it("does not let a late cancel end a recording that has begun", () => {
    const recording: RecorderState = { phase: "recording", id: "r1", startedAt: 1, tracks: ["screen"] };
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
  const recording: RecorderState = { phase: "recording", id: "r1", startedAt: 1, tracks: ["screen"] };

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
