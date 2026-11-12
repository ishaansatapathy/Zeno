import { describe, expect, it } from "vitest";

import { bloomThresholds } from "./bloom";
import { GLYPHS, scrambleFrame } from "./scramble";

describe("scrambleFrame", () => {
  const fixed = () => 0;

  it("is all noise at the start, except for spaces", () => {
    const out = scrambleFrame("Every segment", 0, fixed);
    expect(out).toHaveLength("Every segment".length);
    expect(out[5]).toBe(" ");
    expect(out.replace(" ", "")).toMatch(new RegExp(`^[${GLYPHS.replace(/[-/\\^$*+?.()|[\]{}]/g, "\\$&")}]+$`));
  });

  it("locks characters in from the left", () => {
    expect(scrambleFrame("locked", 0.5, fixed).slice(0, 3)).toBe("loc");
  });

  it("is the target itself when done, and never overshoots", () => {
    expect(scrambleFrame("Teachers.", 1)).toBe("Teachers.");
    expect(scrambleFrame("Teachers.", 7)).toBe("Teachers.");
    expect(scrambleFrame("", 0.4)).toBe("");
  });
});

describe("bloomThresholds", () => {
  const cols = 12;
  const rows = 8;
  const t = bloomThresholds(cols, rows, 6, 4);

  it("gives every cell a threshold between 0 and 1, reaching 1", () => {
    expect(t).toHaveLength(cols * rows);
    expect(Math.min(...t)).toBeGreaterThanOrEqual(0);
    expect(Math.max(...t)).toBe(1);
  });

  it("opens from the centre outwards", () => {
    const centre = t[4 * cols + 6];
    const corner = t[0];
    expect(centre).toBeLessThan(corner);
    expect(centre).toBeLessThan(0.35);
  });

  it("is the same every time", () => {
    expect(bloomThresholds(cols, rows, 6, 4)).toEqual(t);
  });
});
