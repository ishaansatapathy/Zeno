import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";

import { describe, expect, it } from "vitest";

import * as site from "./site";

const components = path.resolve(__dirname, "../components/site");
const source = readdirSync(components)
  .filter((f) => f.endsWith(".tsx"))
  .map((f) => readFileSync(path.join(components, f), "utf8"))
  .join("\n");

/** Every string anywhere in the content module. */
function strings(value: unknown): string[] {
  if (typeof value === "string") return [value];
  if (Array.isArray(value)) return value.flatMap(strings);
  if (value && typeof value === "object") return Object.values(value).flatMap(strings);
  return [];
}

describe("site content", () => {
  it("points every section link at a section that exists", () => {
    for (const item of site.nav) {
      expect(source).toContain(`id="${item.href.slice(1)}"`);
    }
  });

  it("uses each of the six cards exactly once in the deck", () => {
    const images = site.deck.cards.map((c) => c.image).sort();
    expect(images).toEqual([1, 2, 3, 4, 5, 6]);
    expect(new Set(site.deck.cards.map((c) => c.id)).size).toBe(site.deck.cards.length);
  });

  it("describes every card for people who cannot see it", () => {
    for (const c of site.deck.cards) expect(c.alt.length).toBeGreaterThan(20);
  });

  it("is written for teachers everywhere, not one country or currency", () => {
    const text = strings(site).join(" ");
    expect(text).not.toMatch(/india|indian|rupee|₹|\binr\b|paise|gst|razorpay/i);
    expect(text).not.toMatch(/[$€£¥]/);
  });
});
