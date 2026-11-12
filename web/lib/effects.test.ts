import { describe, expect, it } from "vitest";

import { GLYPHS, scrambleFrame } from "./scramble";

describe("scrambleFrame", () => {
  const fixed = () => 0;

  it("is all noise at the start, except for spaces", () => {
    const out = scrambleFrame("Every segment", 0, fixed);
    expect(out).toHaveLength("Every segment".length);
    expect(out[5]).toBe(" ");
    for (const ch of out.replace(" ", "")) expect(GLYPHS).toContain(ch);
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
