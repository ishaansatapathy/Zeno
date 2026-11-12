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
    const anchors = [...site.nav.map((n) => n.href), ...strings(site).filter((s) => s.startsWith("#"))];
    for (const href of new Set(anchors)) {
      expect(source, href).toContain(`id="${href.slice(1)}"`);
    }
  });

  it("walks one lesson through four distinct steps, each with four facts", () => {
    const ids = site.bench.steps.map((s) => s.id);
    expect(new Set(ids).size).toBe(4);
    for (const s of site.bench.steps) {
      expect(s.facts).toHaveLength(4);
      expect(s.stages.length).toBeGreaterThan(0);
    }
  });

  it("states the platform's real limits, not rounder ones", () => {
    const text = strings(site).join(" ");
    expect(text).toContain("144p");
    expect(text).toContain("720p");
    expect(text).not.toMatch(/1080p|4k|unlimited|three devices|3 devices/i);
  });

  it("is written for teachers everywhere, not one country or currency", () => {
    const text = strings(site).join(" ");
    expect(text).not.toMatch(/india|indian|rupee|₹|\binr\b|paise|gst|razorpay/i);
    expect(text).not.toMatch(/[$€£¥]/);
  });
});
