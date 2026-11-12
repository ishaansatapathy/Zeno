import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";

import { describe, expect, it } from "vitest";

type Tree = { [key: string]: string | Tree };

/** `auth.signIn.title` for every string in the tree. */
function flatten(tree: Tree, prefix = ""): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [key, value] of Object.entries(tree)) {
    const at = prefix ? `${prefix}.${key}` : key;
    if (typeof value === "string") out[at] = value;
    else Object.assign(out, flatten(value, at));
  }
  return out;
}

/** The `{placeholders}` in a message, sorted. */
const placeholders = (text: string) => [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();

// Every language file in this folder, so a second language is held to the rules the day it lands.
const files = readdirSync(__dirname).filter((name) => name.endsWith(".json"));
const languages = Object.fromEntries(
  files.map((name) => [
    name.replace(/\.json$/, ""),
    flatten(JSON.parse(readFileSync(path.join(__dirname, name), "utf8")) as Tree),
  ]),
);
const english = languages.en;

describe("every language says the same things as English", () => {
  it("has English, the one the others are measured against", () => {
    expect(english).toBeDefined();
  });

  it.each(Object.keys(languages))("%s has exactly the same messages", (language) => {
    expect(Object.keys(languages[language]).sort()).toEqual(Object.keys(english).sort());
  });

  it.each(Object.keys(languages))("%s leaves nothing empty", (language) => {
    for (const [key, text] of Object.entries(languages[language])) expect(text.trim(), key).not.toBe("");
  });

  it.each(Object.keys(languages))("%s takes the same values: a placeholder missing is a bug", (language) => {
    for (const key of Object.keys(english)) {
      expect(placeholders(languages[language][key]), `${language}: ${key}`).toEqual(placeholders(english[key]));
    }
  });

  it.each(Object.keys(languages).filter((language) => language !== "en"))(
    "%s has not been left in English by accident",
    (language) => {
      // Names such as "Zeno" and "UPI" stay as they are; a whole sentence of English does not.
      for (const [key, text] of Object.entries(languages[language])) {
        if (text.length > 20) expect(text, `${language}: ${key}`).not.toBe(english[key]);
      }
    },
  );
});

describe("the messages themselves", () => {
  it("are whole messages, not the start of something to be glued on to: none ends in a colon or comma", () => {
    for (const [key, text] of Object.entries(english)) {
      expect(text, key).not.toMatch(/[:,]\s*$/);
    }
  });

  it("explain every error the sign-in and sign-up forms can meet", () => {
    for (const code of [
      "invalid_credentials",
      "email_not_verified",
      "validation_failed",
      "too_many_requests",
      "unauthenticated",
      "session_expired",
      "payload_too_large",
      "internal_error",
      "offline",
      "timeout",
      "aborted",
      "unknown",
    ]) {
      expect(english[`errors.${code}`], code).toBeTruthy();
    }
  });
});
