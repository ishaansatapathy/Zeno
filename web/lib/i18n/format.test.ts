import { describe, expect, it } from "vitest";

import { formatDate, formatDateTime, formatNumber, formatRupees } from "./format";

describe("money", () => {
  it("writes rupees the Indian way, without paise when there are none", () => {
    expect(formatRupees(499_900, "en")).toBe("₹4,999");
    expect(formatRupees(49_900, "en")).toBe("₹499");
    expect(formatRupees(0, "en")).toBe("₹0");
    // Lakhs and crores, not thousands and millions.
    expect(formatRupees(12_345_600, "en")).toBe("₹1,23,456");
    expect(formatRupees(1_000_000_000, "en")).toBe("₹1,00,00,000");
  });

  it("shows paise when there are some, always two digits", () => {
    expect(formatRupees(49_950, "en")).toBe("₹499.50");
    expect(formatRupees(5, "en")).toBe("₹0.05");
    expect(formatRupees(100_001, "en")).toBe("₹1,000.01");
  });

  it("handles a negative balance (a refund after a payout leaves one)", () => {
    expect(formatRupees(-1_100, "en")).toBe("-₹11");
    expect(formatRupees(-1_150, "en")).toBe("-₹11.50");
  });

  it("refuses what is not a whole number of paise", () => {
    expect(() => formatRupees(499.5, "en")).toThrow(RangeError);
    expect(() => formatRupees(Number.NaN, "en")).toThrow(RangeError);
    expect(() => formatRupees(Number.MAX_SAFE_INTEGER + 2, "en")).toThrow(RangeError);
  });
});

describe("dates", () => {
  // 18:40 UTC on 31 March is already 1 April, 00:10 in India.
  const lateUtc = "2026-03-31T18:40:00Z";

  it("shows the day it is in India, whatever the machine's time zone", () => {
    expect(formatDate(lateUtc, "en")).toContain("1");
    expect(formatDate(lateUtc, "en")).toMatch(/Apr/);
    expect(formatDateTime(lateUtc, "en")).toMatch(/12:10\s?am/i);
  });

  it("is deterministic: the same moment gives the same text every time", () => {
    expect(formatDateTime(lateUtc, "en")).toBe(formatDateTime(new Date(lateUtc), "en"));
    expect(formatDate(lateUtc, "en")).toBe(formatDate(lateUtc, "en"));
  });
});

describe("numbers", () => {
  it("groups the Indian way", () => {
    expect(formatNumber(1_234_567, "en")).toBe("12,34,567");
  });
});
