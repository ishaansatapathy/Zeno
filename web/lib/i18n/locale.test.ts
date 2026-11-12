import { describe, expect, it } from "vitest";

import { isLocale, negotiate, pickLocale } from "./locale";

// The app has one language today; the rule for choosing is tested with two so that it is right
// the day there are more.
const pick = (cookie: string | undefined, header: string | null | undefined) =>
  negotiate(cookie, header, ["en", "hi"], "en");

describe("which language to show", () => {
  it("honours a choice made on the site over anything the browser says", () => {
    expect(pick("hi", "en-US,en;q=0.9")).toBe("hi");
    expect(pick("en", "hi-IN,hi;q=0.9")).toBe("en");
  });

  it("ignores a cookie that is not a language we have", () => {
    expect(pick("fr", "hi")).toBe("hi");
    expect(pick("", "hi")).toBe("hi");
    expect(pick("HI", "en")).toBe("en");
  });

  it("follows the browser's preference order, region or not", () => {
    expect(pick(undefined, "hi-IN,hi;q=0.9,en;q=0.8")).toBe("hi");
    expect(pick(undefined, "en-GB,en;q=0.9,hi;q=0.8")).toBe("en");
    expect(pick(undefined, "HI")).toBe("hi");
  });

  it("weighs q-values, and skips languages we do not have", () => {
    expect(pick(undefined, "fr;q=0.9,hi;q=0.5,en;q=0.1")).toBe("hi");
    expect(pick(undefined, "en;q=0.2,hi;q=0.8")).toBe("hi");
    expect(pick(undefined, "ta,te,hi;q=0.1")).toBe("hi");
    expect(pick(undefined, "fr,de;q=0.7")).toBe("en");
  });

  it("falls back for anything unusable", () => {
    for (const header of [undefined, null, "", "*", ";;;", "hi;q=0", "hi;q=abc", ",,,"]) {
      expect(pick(undefined, header), String(header)).toBe("en");
    }
  });
});

describe("the app today", () => {
  it("is in English, whatever anyone asks for", () => {
    expect(pickLocale("hi", "hi-IN,hi")).toBe("en");
    expect(pickLocale(undefined, "ta,hi;q=0.5")).toBe("en");
    expect(pickLocale("en", null)).toBe("en");
  });

  it("knows a locale when it sees one", () => {
    expect(isLocale("en")).toBe(true);
    expect(isLocale("EN")).toBe(false);
    expect(isLocale("hi")).toBe(false);
    expect(isLocale(undefined)).toBe(false);
    expect(isLocale(3)).toBe(false);
  });
});
