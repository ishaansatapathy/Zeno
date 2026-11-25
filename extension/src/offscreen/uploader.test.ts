import { afterEach, describe, expect, it, vi } from "vitest";

import type { ApiFailure, Backend, Result } from "../shared/api";
import type { UploadView } from "../shared/messages";
import { backoff, contiguousParts, nextPart, Uploader, type Part, type Source } from "./uploader";

describe("which part is next", () => {
  it("sends only full parts while recording", () => {
    expect(nextPart(0, 10, 0, false)).toBeNull();
    expect(nextPart(9, 10, 0, false)).toBeNull();
    expect(nextPart(10, 10, 0, false)).toEqual({ number: 1, start: 0, end: 10, last: false });
    expect(nextPart(25, 10, 1, false)).toEqual({ number: 2, start: 10, end: 20, last: false });
    expect(nextPart(25, 10, 2, false)).toBeNull();
  });

  it("sends what is left as the last part once the recording has ended, however short", () => {
    expect(nextPart(25, 10, 2, true)).toEqual({ number: 3, start: 20, end: 25, last: true });
    expect(nextPart(1, 10, 0, true)).toEqual({ number: 1, start: 0, end: 1, last: true });
  });

  it("marks a full part as the last one only when it really is", () => {
    expect(nextPart(20, 10, 0, true)).toEqual({ number: 1, start: 0, end: 10, last: false });
    expect(nextPart(20, 10, 1, true)).toEqual({ number: 2, start: 10, end: 20, last: true });
    expect(nextPart(20, 10, 2, true)).toBeNull();
  });

  it("covers every byte exactly once, for any size", () => {
    for (const size of [1, 9, 10, 11, 19, 20, 21, 95, 100]) {
      const parts: Part[] = [];
      for (let part = nextPart(size, 10, 0, true); part; part = nextPart(size, 10, parts.length, true)) parts.push(part);
      expect(parts[0]?.start).toBe(0);
      expect(parts.at(-1)?.end).toBe(size);
      parts.slice(1).forEach((part, i) => expect(part.start).toBe(parts[i]?.end));
      expect(parts.filter((part) => part.last)).toHaveLength(1);
      expect(parts.at(-1)?.last).toBe(true);
    }
  });
});

describe("waiting between tries", () => {
  it("doubles from half a second up to fifteen", () => {
    expect([0, 1, 2, 3, 4, 5, 6, 10].map(backoff)).toEqual([500, 1000, 2000, 4000, 8000, 15000, 15000, 15000]);
  });
});

// ---- a fake Zeno, and a fake disk ---------------------------------------------------------------------

const ok = <T>(data: T): Result<T> => ({ ok: true, data });
const fail = (status: number, code = "x"): Result<never> => ({ ok: false, error: { status, code, message: `no (${status})` } satisfies ApiFailure });

class FakeZeno implements Backend {
  calls: string[] = [];
  startIds: string[] = [];
  linkIds: string[] = [];
  parts = new Map<number, Uint8Array>();
  completedWith: number | null = null;
  /** Answers to give before the real one, per call name: a status number is a failure (with its own code, if given). */
  script: Record<string, (number | [number, string])[]> = {};
  /** The numbers of the parts storage is said to hold. */
  held: number[] = [];

  private scripted(name: string): Result<never> | null {
    this.calls.push(name);
    const next = this.script[name]?.shift();
    if (next === undefined) return null;
    return Array.isArray(next) ? fail(next[0], next[1]) : fail(next);
  }

  async me() {
    return this.scripted("me") ?? ok({ id: "u", name: "Asha", email: "a@x.in", email_verified: true, role: "creator" as const });
  }
  async startUpload(input: { id: string }) {
    this.startIds.push(input.id);
    return (
      this.scripted("startUpload") ??
      ok({
        video: { id: "video-1" } as never,
        upload: { part_size: 10, total_parts: 0, uploaded: [], parts: [], expires_at: "" },
      })
    );
  }
  async uploadProgress() {
    return this.scripted("uploadProgress") ?? ok({ part_size: 10, total_parts: 0, uploaded: this.held, parts: [], expires_at: "" });
  }
  async createRecording() {
    return this.scripted("createRecording") ?? ok({ id: "rec-1" } as never);
  }
  async createLink(recordingId: string) {
    this.linkIds.push(recordingId);
    return this.scripted("createLink") ?? ok({ url: "https://zeno.test/s/abc" } as never);
  }
  async partUrl(_video: string, part: { number: number; size: number; last: boolean }) {
    return this.scripted("partUrl") ?? ok({ part: { number: part.number, size: part.size, url: `https://storage.test/${part.number}` }, expires_at: "" });
  }
  async complete(_video: string, size: number) {
    const refused = this.scripted("complete");
    if (refused) return refused;
    this.completedWith = size;
    return ok({ video: { id: "video-1" } as never });
  }
}

class FakeDisk implements Source {
  bytes = new Uint8Array(0);
  append(count: number): void {
    const next = new Uint8Array(this.bytes.length + count);
    next.set(this.bytes);
    for (let i = this.bytes.length; i < next.length; i += 1) next[i] = i % 251;
    this.bytes = next;
  }
  size() {
    return this.bytes.length;
  }
  async read(start: number, end: number) {
    return this.bytes.slice(start, end).buffer;
  }
}

function setup(zeno = new FakeZeno(), disk = new FakeDisk(), resume = false) {
  const views: UploadView[] = [];
  const sleeps: number[] = [];
  const sent = new Map<string, Uint8Array>();
  let putFailures = 0;
  const uploader = new Uploader(
    {
      backend: zeno,
      source: disk,
      put: async (url, body) => {
        if (putFailures > 0) {
          putFailures -= 1;
          throw new Error("storage answered 503");
        }
        sent.set(url, new Uint8Array(body));
      },
      sleep: async (ms) => {
        sleeps.push(ms);
      },
      emit: (view) => views.push(view),
    },
    { recordingId: "rec-1", title: "Recording", filename: "r.webm", partSize: 10, resume },
  );
  return { zeno, disk, uploader, views, sleeps, sent, failPuts: (n: number) => (putFailures = n) };
}

/** Lets the uploader's own loop run (it waits on timers and promises). */
const settle = () => new Promise((resolve) => setTimeout(resolve, 20));

afterEach(() => vi.useRealTimers());

describe("uploading while recording", () => {
  it("has the link ready before the recording ends, and sends each part as soon as it is full", async () => {
    const { zeno, disk, uploader, views, sent } = setup();
    uploader.start();
    await settle();
    expect(views.at(-1)).toMatchObject({ phase: "uploading", url: "https://zeno.test/s/abc", total: null });

    disk.append(25);
    uploader.notify();
    await settle();
    expect([...sent.keys()]).toEqual(["https://storage.test/1", "https://storage.test/2"]);
    expect(views.at(-1)).toMatchObject({ phase: "uploading", sent: 20 });
    expect(zeno.completedWith, "not finished while the recording goes on").toBeNull();

    const finished = await uploader.end(disk.size());
    expect(finished).toMatchObject({ phase: "done", sent: 25, total: 25, url: "https://zeno.test/s/abc" });
    expect(zeno.completedWith).toBe(25);
    expect([...sent.keys()]).toHaveLength(3);
    // What storage holds is the recording, byte for byte.
    const joined = [1, 2, 3].flatMap((n) => [...(sent.get(`https://storage.test/${n}`) ?? [])]);
    expect(joined).toEqual([...disk.bytes]);
  });

  it("leaves only what has not been sent for the end, so stopping is quick", async () => {
    const { zeno, disk, uploader } = setup();
    uploader.start();
    disk.append(30);
    uploader.notify();
    await settle();
    zeno.calls.length = 0;
    disk.append(4);
    await uploader.end(disk.size());
    // Part 4 only: nothing is asked or sent again for parts 1-3.
    expect(zeno.calls.filter((call) => call === "partUrl")).toHaveLength(1);
    expect(zeno.calls.at(-1)).toBe("complete");
  });

  it("ends a recording that was exactly whole parts", async () => {
    const { zeno, disk, uploader, sent } = setup();
    uploader.start();
    disk.append(20);
    uploader.notify();
    await settle();
    await uploader.end(20);
    expect([...sent.keys()]).toHaveLength(2);
    expect(zeno.completedWith).toBe(20);
  });
});

describe("when the network or Zeno misbehaves", () => {
  it("keeps recording-side work going and catches up: a part that fails is tried again, never skipped or doubled", async () => {
    const { zeno, disk, uploader, sleeps, sent } = setup();
    zeno.script["partUrl"] = [0, 503];
    uploader.start();
    disk.append(10);
    uploader.notify();
    await settle();
    expect(sent.size).toBe(1);
    expect(sleeps).toEqual([500, 1000]);
    expect(zeno.calls.filter((call) => call === "partUrl")).toHaveLength(3);
  });

  it("tries a part again when storage refuses it, with a fresh address each time", async () => {
    const { zeno, disk, uploader, failPuts, sent } = setup();
    failPuts(2);
    uploader.start();
    disk.append(10);
    uploader.notify();
    await settle();
    expect(sent.size).toBe(1);
    expect(zeno.calls.filter((call) => call === "partUrl")).toHaveLength(3);
  });

  it("waits for Zeno to come back at the start, and says so", async () => {
    const { zeno, uploader, views } = setup();
    zeno.script["startUpload"] = [0, 0, 502];
    uploader.start();
    await settle();
    expect(views.some((view) => view.phase === "connecting" && view.reason === "unreachable")).toBe(true);
    expect(views.at(-1)).toMatchObject({ phase: "uploading", reason: undefined, url: "https://zeno.test/s/abc" });
    expect(zeno.startIds).toEqual(["rec-1", "rec-1", "rec-1", "rec-1"]);
  });

  it("retries a lost link response under the same client id", async () => {
    const { zeno, uploader, views } = setup();
    zeno.script["createLink"] = [0];
    uploader.start();
    await settle();
    expect(zeno.linkIds).toEqual(["rec-1", "rec-1"]);
    expect(views.at(-1)?.url).toBe("https://zeno.test/s/abc");
  });

  it("does not upload for someone who is signed out, and says why", async () => {
    const { zeno, disk, uploader } = setup();
    zeno.script["me"] = [401];
    uploader.start();
    disk.append(50);
    const view = await uploader.end(50);
    expect(view).toMatchObject({ phase: "off", reason: "signed_out", url: null });
    expect(zeno.calls).toEqual(["me"]);
  });

  it("does not call an account that may not upload a failure", async () => {
    const { zeno, uploader } = setup();
    zeno.script["startUpload"] = [403];
    uploader.start();
    const view = await uploader.end(5);
    expect(view).toMatchObject({ phase: "off", reason: "refused" });
  });

  it("says plainly when Zeno has no room, at the start and part way through, and goes on recording", async () => {
    const atStart = setup();
    atStart.zeno.script["startUpload"] = [[413, "storage_quota_exceeded"]];
    atStart.uploader.start();
    atStart.disk.append(30);
    const first = await atStart.uploader.end(30);
    expect(first).toMatchObject({ phase: "failed" });
    expect(first.message).toContain("storage is full");
    expect(first.message).not.toContain("413");

    // Part of it is in Zeno when the room runs out: what is held is not thrown away, and the account may try again.
    const partWay = setup();
    partWay.zeno.script["partUrl"] = [0, [413, "storage_quota_exceeded"]];
    partWay.uploader.start();
    partWay.disk.append(30);
    const second = await partWay.uploader.end(30);
    expect(second).toMatchObject({ phase: "failed" });
    expect(second.message).toContain("storage is full");
    expect(partWay.zeno.calls.filter((call) => call === "partUrl")).toHaveLength(2);
    // It stopped: no more asks after a refusal that waiting cannot fix.
    expect(partWay.zeno.calls.at(-1)).toBe("partUrl");
  });

  it("stops at a refusal that waiting cannot fix", async () => {
    const { zeno, disk, uploader } = setup();
    zeno.script["complete"] = [409];
    uploader.start();
    disk.append(10);
    const view = await uploader.end(10);
    expect(view.phase).toBe("failed");
    expect(view.message).toContain("409");
  });

  it("gives up some time after the recording has ended, if Zeno never comes back", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(0);
    const { zeno, disk, uploader, views } = setup();
    // Time passes only as the uploader waits.
    const clock = { now: 0 };
    zeno.script["startUpload"] = Array.from({ length: 500 }, () => 0);
    const original = zeno.startUpload.bind(zeno);
    zeno.startUpload = async (input) => {
      clock.now += 10_000;
      vi.setSystemTime(clock.now);
      return original(input);
    };
    uploader.start();
    disk.append(10);
    const view = await uploader.end(10);
    expect(view.phase).toBe("failed");
    expect(views.at(-1)?.message).toMatch(/could not reach zeno/i);
  });
});

describe("which parts storage holds", () => {
  it("counts from the first, without a gap", () => {
    expect(contiguousParts([])).toBe(0);
    expect(contiguousParts([1, 2, 3])).toBe(3);
    expect(contiguousParts([3, 1, 2])).toBe(3);
    expect(contiguousParts([2, 3])).toBe(0);
    expect(contiguousParts([1, 2, 4, 5])).toBe(2);
  });
});

describe("finishing a recording that an earlier run left", () => {
  /** A whole recording on disk (it has ended), as it is found after the browser came back. */
  function finishedOnDisk(bytes: number, held: number[], script: Record<string, (number | [number, string])[]> = {}) {
    const zeno = new FakeZeno();
    zeno.held = held;
    zeno.script = script;
    const disk = new FakeDisk();
    disk.append(bytes);
    const made = setup(zeno, disk, true);
    made.uploader.start();
    return { ...made, finished: made.uploader.end(bytes) };
  }

  it("sends only what storage does not hold, under the same ids as before, and gets the same link", async () => {
    const { zeno, finished, sent, disk } = finishedOnDisk(25, [1, 2]);
    expect(await finished).toMatchObject({ phase: "done", sent: 25, total: 25, url: "https://zeno.test/s/abc" });
    expect(zeno.startIds).toEqual(["rec-1"]);
    expect(zeno.linkIds).toEqual(["rec-1"]);
    // Part 3 only: nothing is asked for or sent again for parts 1 and 2.
    expect(zeno.calls.filter((call) => call === "partUrl")).toHaveLength(1);
    expect([...sent.keys()]).toEqual(["https://storage.test/3"]);
    expect([...(sent.get("https://storage.test/3") ?? [])]).toEqual([...disk.bytes.slice(20)]);
    expect(zeno.completedWith).toBe(25);
  });

  it("starts again from a gap, because skipping a part that is missing would leave a hole", async () => {
    const { zeno, finished, sent } = finishedOnDisk(35, [1, 3]);
    await finished;
    expect([...sent.keys()]).toEqual(["https://storage.test/2", "https://storage.test/3", "https://storage.test/4"]);
    expect(zeno.completedWith).toBe(35);
  });

  it("sends everything when storage holds nothing", async () => {
    const { zeno, finished, sent } = finishedOnDisk(20, []);
    expect(await finished).toMatchObject({ phase: "done" });
    expect(sent.size).toBe(2);
    expect(zeno.completedWith).toBe(20);
  });

  it("leaves a resumed upload retryable if sign-in or permission is missing", async () => {
    const signedOut = finishedOnDisk(20, [1], { me: [401] });
    expect(await signedOut.finished).toMatchObject({ phase: "failed", reason: "signed_out" });
    expect(signedOut.zeno.calls).toEqual(["me"]);
    const refused = finishedOnDisk(20, [1], { startUpload: [403] });
    expect(await refused.finished).toMatchObject({ phase: "failed", reason: "refused" });
    expect(refused.sent.size).toBe(0);
  });

  it("says it is done when the whole recording was already in Zeno, which is for Zeno to confirm", async () => {
    // The earlier run got as far as assembling it: the upload is no longer open.
    const { zeno, finished, sent } = finishedOnDisk(25, [], { startUpload: [[409, "streaming_id_conflict"]] });
    expect(await finished).toMatchObject({ phase: "done", url: "https://zeno.test/s/abc" });
    expect(sent.size).toBe(0);
    expect(zeno.calls).not.toContain("partUrl");
    expect(zeno.calls).not.toContain("uploadProgress");
    expect(zeno.completedWith).toBe(25);
  });

  it("does the same when the upload turns out to be closed after it was started again", async () => {
    const { zeno, finished } = finishedOnDisk(25, [], { uploadProgress: [[409, "not_uploading"]] });
    expect(await finished).toMatchObject({ phase: "done" });
    expect(zeno.calls).not.toContain("partUrl");
    expect(zeno.completedWith).toBe(25);
  });

  it("does not call a recording whole when Zeno says it is not", async () => {
    const { finished } = finishedOnDisk(25, [], { startUpload: [[409, "streaming_id_conflict"]], complete: [[409, "upload_incomplete"]] });
    const view = await finished;
    expect(view.phase).toBe("failed");
    expect(view.message).toContain("409");
  });

  it("waits for Zeno to come back when it asks what is held", async () => {
    const { zeno, finished, sleeps } = finishedOnDisk(20, [1], { uploadProgress: [0, 503] });
    expect(await finished).toMatchObject({ phase: "done" });
    expect(sleeps).toEqual([500, 1000]);
    expect(zeno.calls.filter((call) => call === "uploadProgress")).toHaveLength(3);
  });

  it("does not ask what is held for a recording that is being made now", async () => {
    const { zeno, disk, uploader } = setup();
    uploader.start();
    disk.append(10);
    await uploader.end(10);
    expect(zeno.calls).not.toContain("uploadProgress");
  });

  it("only treats a closed upload as finished when it was told to pick one up", async () => {
    // A new recording must never be told "already done": a clash of ids there is an error.
    const zeno = new FakeZeno();
    zeno.script["startUpload"] = [[409, "streaming_id_conflict"]];
    const { uploader, disk } = setup(zeno);
    uploader.start();
    disk.append(10);
    const view = await uploader.end(10);
    expect(view.phase).toBe("failed");
    expect(zeno.completedWith).toBeNull();
  });
});
