import { describe, expect, it } from "vitest";

import { AFTER_SIGN_IN, safeNext } from "./next-path";

describe("safeNext", () => {
  it("keeps a path on this site, with its query", () => {
    expect(safeNext("/library")).toBe("/library");
    expect(safeNext("/library/abc?tab=links#x")).toBe("/library/abc?tab=links#x");
  });

  it("falls back when there is nothing, or too much", () => {
    expect(safeNext(undefined)).toBe(AFTER_SIGN_IN);
    expect(safeNext("")).toBe(AFTER_SIGN_IN);
    expect(safeNext(`/${"a".repeat(600)}`)).toBe(AFTER_SIGN_IN);
  });

  it("uses the first when the parameter is repeated", () => {
    expect(safeNext(["/library", "https://evil.example"])).toBe("/library");
    expect(safeNext(["https://evil.example", "/library"])).toBe(AFTER_SIGN_IN);
  });

  const back = String.fromCharCode(92);
  it.each([
    "https://evil.example",
    "//evil.example",
    `/${back}evil.example`,
    `${back}${back}evil.example`,
    "javascript:alert(1)",
    "evil.example",
    `/${String.fromCharCode(9)}evil`,
    `/a${String.fromCharCode(10)}b`,
    `/${String.fromCharCode(0)}`,
  ])("refuses %j", (value) => {
    expect(safeNext(value)).toBe(AFTER_SIGN_IN);
  });

  it("leaves an encoded character alone: the browser, not this function, decodes it", () => {
    expect(safeNext("/library%0a")).toBe("/library%0a");
  });

  it("does not send someone back to a sign-in page", () => {
    for (const page of ["/login", "/login?next=/library", "/register", "/forgot-password", "/verify-email/abc", "/reset-password/abc"]) {
      expect(safeNext(page)).toBe(AFTER_SIGN_IN);
    }
    expect(safeNext("/login-help")).toBe("/login-help");
  });
});
