import { describe, expect, it } from "vitest";

import { closeOff, expired, interruptedLength, judge, KEEP_EMPTY_MS, KEEP_FINISHED_MS, toUnfinished, type Manifest, type Stored, type UploadFile } from "./recovery";

const DAY = 24 * 3600 * 1000;

function manifest(over: Partial<Manifest> = {}): Manifest {
  return { version: 1, id: "r1", status: "recording", startedAt: 1_000_000, tracks: [{ track: "screen", file: "screen.webm", mime: "video/webm", startedAt: 1_000_100 }], ...over };
}
function upload(status: UploadFile["status"]): UploadFile {
  return { version: 1, status, partSize: 5 * 1024 * 1024 };
}
function stored(over: Partial<Stored> = {}): Stored {
  return { id: "r1", manifest: manifest(), upload: null, bytes: 50_000, lastWriteAt: 1_061_000, ...over };
}

describe("what became of a recording", () => {
  it("leaves the one being recorded alone", () => {
    expect(judge(stored(), "r1")).toBe("live");
    expect(judge(stored(), "someone-else")).toBe("interrupted");
  });

  it("calls a recording that was never stopped interrupted, if there is anything in it", () => {
    expect(judge(stored(), null)).toBe("interrupted");
    expect(judge(stored({ bytes: 0 }), null)).toBe("empty");
  });

  it("calls a stopped recording whose upload did not finish an upload to finish, and only when one was meant", () => {
    const done = stored({ manifest: manifest({ status: "complete" }) });
    expect(judge({ ...done, upload: upload("pending") }, null)).toBe("upload");
    expect(judge({ ...done, upload: upload("failed") }, null)).toBe("upload");
    expect(judge({ ...done, upload: upload("done") }, null)).toBe("finished");
    expect(judge({ ...done, upload: upload("skipped") }, null)).toBe("finished");
    // Made before uploads were recorded at all: nothing says it was meant to go anywhere.
    expect(judge({ ...done, upload: null }, null)).toBe("finished");
  });

  it("does not guess about what it cannot read", () => {
    expect(judge(stored({ manifest: null }), null)).toBe("unknown");
    expect(judge(stored({ manifest: null, bytes: 0 }), null)).toBe("empty");
  });
});

describe("what may be deleted", () => {
  const done = (over: Partial<Stored> = {}) => stored({ manifest: manifest({ status: "complete", endedAt: 2_000_000 }), upload: upload("done"), ...over });

  it("is a finished recording once it is a week old, and not before", () => {
    expect(expired(done(), null, 2_000_000 + KEEP_FINISHED_MS, true)).toBe(false);
    expect(expired(done(), null, 2_000_000 + KEEP_FINISHED_MS + 1, true)).toBe(true);
  });

  it("is an empty one after a day", () => {
    const empty = stored({ bytes: 0, lastWriteAt: 5_000_000, emptyVerified: true });
    expect(expired(empty, null, 5_000_000 + KEEP_EMPTY_MS)).toBe(false);
    expect(expired(empty, null, 5_000_000 + KEEP_EMPTY_MS + 1)).toBe(true);
  });

  it("is never a recording that could still be finished, however old", () => {
    const old = 1_000_000 + 400 * DAY;
    expect(expired(stored(), null, old)).toBe(false);
    expect(expired(done({ upload: upload("pending") }), null, old)).toBe(false);
    expect(expired(done({ upload: upload("failed") }), null, old)).toBe(false);
    expect(expired(stored({ manifest: null }), null, old)).toBe(false);
  });

  it("is never the recording being made", () => {
    expect(expired(stored({ bytes: 0 }), "r1", 1_000_000 + 400 * DAY)).toBe(false);
    expect(expired(done(), "r1", 1_000_000 + 400 * DAY)).toBe(false);
  });

  it("keeps the file when the download cannot be confirmed, and offers to save it", () => {
    const recording = done({ upload: upload("skipped") });
    expect(expired(recording, null, 2_000_000 + 30 * DAY)).toBe(false);
    expect(toUnfinished(recording, null, true)?.kind).toBe("download");
    expect(expired(stored({ bytes: 0 }), null, 30 * DAY)).toBe(false);
  });
});

describe("what is offered to the person", () => {
  it("describes a recording that was cut short by how long it ran, to the last thing written", () => {
    const item = toUnfinished(stored(), null);
    expect(item).toEqual({ id: "r1", startedAt: 1_000_000, durationMs: 61_000, bytes: 50_000, kind: "interrupted" });
  });

  it("does not count the time it was paused", () => {
    const paused = stored({ manifest: manifest({ pauses: [{ from: 1_010_000, to: 1_040_000 }] }) });
    expect(toUnfinished(paused, null)?.durationMs).toBe(31_000);
  });

  it("describes a stopped recording with an unfinished upload by its recorded length", () => {
    const item = toUnfinished(stored({ manifest: manifest({ status: "complete", durationMs: 90_000, endedAt: 1_090_000 }), upload: upload("failed") }), null);
    expect(item).toMatchObject({ kind: "upload", durationMs: 90_000 });
  });

  it("offers nothing for what is done, empty, unreadable or live", () => {
    expect(toUnfinished(stored({ manifest: manifest({ status: "complete" }), upload: upload("done") }), null)).toBeNull();
    expect(toUnfinished(stored({ bytes: 0 }), null)).toBeNull();
    expect(toUnfinished(stored({ manifest: null }), null)).toBeNull();
    expect(toUnfinished(stored(), "r1")).toBeNull();
  });

  it("never gives a negative length when the clock was set back", () => {
    expect(interruptedLength(manifest({ startedAt: 5000 }), 1000)).toBe(0);
  });
});

describe("closing off a recording that was cut short", () => {
  it("says what the manifest would have said had it been stopped, ending where the last write did", () => {
    const closed = closeOff(manifest({ pauses: [{ from: 1_010_000, to: 1_020_000 }] }), 1_061_000, [50_000]);
    expect(closed).toMatchObject({ status: "complete", endedAt: 1_061_000, durationMs: 51_000, recovered: true, bytes: [50_000] });
    expect(closed.tracks).toHaveLength(1);
  });

  it("ends a pause that was still going on where the recording ends, and counts it as paused", () => {
    const closed = closeOff(manifest({ pauses: [{ from: 1_040_000, to: null }] }), 1_061_000, [1]);
    expect(closed.pauses).toEqual([{ from: 1_040_000, to: 1_061_000 }]);
    expect(closed.durationMs).toBe(40_000);
  });

  it("keeps everything else the manifest said", () => {
    const closed = closeOff(manifest(), 1_061_000, [1]);
    expect(closed).toMatchObject({ id: "r1", startedAt: 1_000_000, version: 1 });
  });
});
