import { describe, expect, it } from "vitest";

import en from "@/messages/en.json";

import { looksLikeEmail, validateSignIn, validateSignUp } from "./validation";

describe("sign in", () => {
  it("asks for both fields", () => {
    expect(validateSignIn({ email: "", password: "" })).toEqual({ email: "required", password: "required" });
    expect(validateSignIn({ email: "   ", password: "x" })).toEqual({ email: "required" });
  });

  it("accepts anything that looks like an address, and leaves the password to the server", () => {
    expect(validateSignIn({ email: "asha@example.in", password: "x" })).toEqual({});
    expect(validateSignIn({ email: " asha@example.in ", password: "x" })).toEqual({});
    expect(validateSignIn({ email: "asha@example", password: "x" })).toEqual({ email: "email" });
  });
});

describe("sign up", () => {
  const good = { name: "Asha Rao", email: "asha@example.in", password: "a long enough one" };

  it("accepts a good form", () => {
    expect(validateSignUp(good)).toEqual({});
  });

  it("holds the password to the API's length rule, counting characters as people do", () => {
    expect(validateSignUp({ ...good, password: "123456789" }).password).toBe("passwordMin");
    expect(validateSignUp({ ...good, password: "1234567890" }).password).toBeUndefined();
    // Ten characters in Devanagari and in emoji are ten characters, whatever their byte length.
    expect(validateSignUp({ ...good, password: "पासवर्डपासवर्ड" }).password).toBeUndefined();
    expect(validateSignUp({ ...good, password: "🙂".repeat(9) }).password).toBe("passwordMin");
    expect(validateSignUp({ ...good, password: "x".repeat(129) }).password).toBe("passwordMax");
    expect(validateSignUp({ ...good, password: "x".repeat(128) }).password).toBeUndefined();
  });

  it("holds the name to 2 to 80 characters", () => {
    expect(validateSignUp({ ...good, name: "A" }).name).toBe("nameLength");
    expect(validateSignUp({ ...good, name: "आ" }).name).toBe("nameLength");
    expect(validateSignUp({ ...good, name: "आशा" }).name).toBeUndefined();
    expect(validateSignUp({ ...good, name: "n".repeat(81) }).name).toBe("nameLength");
    expect(validateSignUp({ ...good, name: "  " }).name).toBe("required");
  });

  it("reports every problem at once, so the person fixes them in one go", () => {
    expect(validateSignUp({ name: "", email: "nope", password: "short" })).toEqual({
      name: "required",
      email: "email",
      password: "passwordMin",
    });
  });
});

describe("email shape", () => {
  it("is generous about what is allowed and strict about what is missing", () => {
    for (const ok of ["a@b.co", "first.last+tag@sub.example.in", "ए@उदाहरण.भारत"]) {
      expect(looksLikeEmail(ok), ok).toBe(true);
    }
    for (const bad of ["", "a", "a@b", "a@b.c", "@b.co", "a b@c.co", "a@@b.co", `${"x".repeat(250)}@b.co`]) {
      expect(looksLikeEmail(bad), bad).toBe(false);
    }
  });
});

describe("the words for each problem", () => {
  it("exist", () => {
    for (const key of ["required", "email", "passwordMin", "passwordMax", "nameLength"]) {
      expect((en.validation as Record<string, string>)[key], key).toBeTruthy();
    }
  });
});
