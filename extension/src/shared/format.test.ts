import { describe, expect, it } from "vitest";

import { bytesText, clock, fileName, pickMime } from "./format";

describe("clock", () => {
  it("writes minutes and seconds, and hours only when there are some", () => {
    expect(clock(0)).toBe("0:00");
    expect(clock(999)).toBe("0:00");
    expect(clock(83_000)).toBe("1:23");
    expect(clock(3_723_000)).toBe("1:02:03");
  });

  it("never goes negative", () => {
    expect(clock(-5000)).toBe("0:00");
  });
});

describe("bytesText", () => {
  it("uses the unit a person would", () => {
    expect(bytesText(0)).toBe("0 B");
    expect(bytesText(1023)).toBe("1023 B");
    expect(bytesText(1536)).toBe("1.5 KB");
    expect(bytesText(5 * 1024 * 1024)).toBe("5.0 MB");
    expect(bytesText(3.2 * 1024 ** 3)).toBe("3.2 GB");
  });
});

describe("fileName", () => {
  it("sorts by time, names the track, and uses nothing a file system refuses", () => {
    const at = new Date(2026, 10, 21, 14, 5, 9).getTime();
    expect(fileName("screen", at)).toBe("Zeno 2026-11-21 14.05.09 screen.webm");
    expect(fileName("camera", at)).toBe("Zeno 2026-11-21 14.05.09 camera.webm");
    expect(fileName("screen", at)).not.toMatch(/[<>:"/\\|?*]/);
  });

  it("puts the files of one recording together and in time order", () => {
    const early = fileName("screen", new Date(2026, 10, 21, 9, 0, 0).getTime());
    const late = fileName("camera", new Date(2026, 10, 21, 18, 0, 0).getTime());
    expect([late, early].sort()).toEqual([early, late]);
  });
});

describe("pickMime", () => {
  it("takes the best the browser supports", () => {
    expect(pickMime(() => true)).toBe("video/webm;codecs=vp9,opus");
    expect(pickMime((m) => m === "video/webm;codecs=vp8,opus" || m === "video/webm")).toBe("video/webm;codecs=vp8,opus");
    expect(pickMime((m) => m === "video/webm")).toBe("video/webm");
  });

  it("says so when it supports none", () => {
    expect(pickMime(() => false)).toBeNull();
  });
});
