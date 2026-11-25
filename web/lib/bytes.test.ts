import { describe, expect, it } from "vitest";

import { formatBytes, percentUsed } from "./bytes";

describe("formatBytes", () => {
  it("says sizes the way the API does", () => {
    expect(formatBytes(0)).toBe("0 bytes");
    expect(formatBytes(1023)).toBe("1023 bytes");
    expect(formatBytes(1024)).toBe("1.0 KB");
    expect(formatBytes(5 * 1024 * 1024)).toBe("5.0 MB");
    expect(formatBytes(25 * 1024 ** 3)).toBe("25.0 GB");
    expect(formatBytes(1536 * 1024 * 1024)).toBe("1.5 GB");
  });

  it("does not break on what no size can be", () => {
    expect(formatBytes(-5)).toBe("0 bytes");
    expect(formatBytes(Number.NaN)).toBe("0 bytes");
    expect(formatBytes(Number.POSITIVE_INFINITY)).toBe("0 bytes");
    expect(formatBytes(2 ** 60)).toMatch(/TB$/);
  });
});

describe("percentUsed", () => {
  it("is a whole percent of the allowance", () => {
    expect(percentUsed(0, 100)).toBe(0);
    expect(percentUsed(50, 100)).toBe(50);
    expect(percentUsed(100, 100)).toBe(100);
  });

  it("shows any use at all, and never more than all", () => {
    expect(percentUsed(1, 1_000_000)).toBe(1);
    expect(percentUsed(300, 100)).toBe(100);
  });

  it("copes with an allowance of nothing", () => {
    expect(percentUsed(0, 0)).toBe(0);
    expect(percentUsed(1, 0)).toBe(100);
  });
});
