import { describe, expect, it } from "vitest";

import { limits } from "@/lib/api/limits";

import { checkEmail, checkLogin, checkName, checkNewPassword, checkRegister, checkReset, firstProblem } from "./validate";

describe("limits come from the API's own document", () => {
  it("are the ones the server enforces", () => {
    expect(limits).toEqual({ passwordMin: 10, passwordMax: 128, nameMin: 2, nameMax: 80, titleMax: 120 });
  });
});

describe("checkName", () => {
  it("counts what a person sees, after trimming", () => {
    expect(checkName("  A  ")).not.toBeNull();
    expect(checkName(" Al ")).toBeNull();
    expect(checkName("x".repeat(80))).toBeNull();
    expect(checkName("x".repeat(81))).not.toBeNull();
  });

  it("counts a character outside the basic plane once, as the server does", () => {
    expect(checkName("\u{1F600}\u{1F600}")).toBeNull();
    expect(checkName("x".repeat(79) + "\u{1F600}")).toBeNull();
  });
});

describe("checkEmail", () => {
  it("accepts an address and rejects what is plainly not one", () => {
    expect(checkEmail(" asha@example.com ")).toBeNull();
    for (const bad of ["", "   ", "asha", "asha@", "@example.com", "asha@example", "a b@example.com"]) {
      expect(checkEmail(bad), bad).not.toBeNull();
    }
  });
});

describe("checkNewPassword", () => {
  it("enforces the length the server does, in characters", () => {
    expect(checkNewPassword("x".repeat(9))).toMatch(/at least 10/);
    expect(checkNewPassword("x".repeat(10))).toBeNull();
    expect(checkNewPassword("x".repeat(128))).toBeNull();
    expect(checkNewPassword("x".repeat(129))).toMatch(/at most 128/);
    expect(checkNewPassword("\u{1F600}".repeat(10))).toBeNull();
  });
});

describe("forms", () => {
  it("report every problem at once, keyed by the API's field names", () => {
    expect(checkRegister({ name: "x", email: "nope", password: "short" })).toEqual({
      name: expect.any(String),
      email: expect.any(String),
      password: expect.any(String),
    });
    expect(checkRegister({ name: "Asha", email: "a@b.co", password: "a long enough one" })).toEqual({});
  });

  it("do not set a minimum for signing in: an old password must still work", () => {
    expect(checkLogin({ email: "a@b.co", password: "short" })).toEqual({});
    expect(checkLogin({ email: "a@b.co", password: "" })).toHaveProperty("password");
  });

  it("make sure the new password was typed twice the same", () => {
    expect(checkReset({ password: "a long enough one", confirm: "a long enough one" })).toEqual({});
    expect(checkReset({ password: "a long enough one", confirm: "a long enough on" })).toHaveProperty("confirm");
  });

  it("find the first field in screen order", () => {
    const problems = { password: "p", email: "e" };
    expect(firstProblem(problems, ["name", "email", "password"])).toBe("email");
    expect(firstProblem({ other: "o" }, ["name"])).toBe("other");
  });
});
