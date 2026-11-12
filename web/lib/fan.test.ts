import { describe, expect, it } from "vitest";

import { ringOffset, slot, wrap } from "./fan";

describe("ringOffset", () => {
  it("is zero for the active card", () => {
    expect(ringOffset(3, 3, 6)).toBe(0);
  });

  it("takes the short way round the ring", () => {
    expect(ringOffset(5, 0, 6)).toBe(-1);
    expect(ringOffset(1, 5, 6)).toBe(2);
    expect(ringOffset(0, 4, 5)).toBe(1);
  });

  it("puts the card opposite on the right in an even deck", () => {
    expect(ringOffset(3, 0, 6)).toBe(3);
    expect(ringOffset(0, 3, 6)).toBe(3);
  });

  it("gives every card a distinct offset", () => {
    for (const count of [1, 2, 5, 6, 7]) {
      for (let active = 0; active < count; active++) {
        const offsets = Array.from({ length: count }, (_, i) => ringOffset(i, active, count));
        expect(new Set(offsets).size).toBe(count);
      }
    }
  });
});

describe("wrap", () => {
  it("brings any index back into the deck", () => {
    expect(wrap(-1, 6)).toBe(5);
    expect(wrap(13, 6)).toBe(1);
    expect(wrap(0, 6)).toBe(0);
  });
});

describe("slot", () => {
  it("stands the active card upright, in front, full size", () => {
    expect(slot(2, 2, 6)).toMatchObject({ offset: 0, x: 0, rotate: 0, scale: 1, z: 10, visible: true });
  });

  it("mirrors the fan on both sides", () => {
    const left = slot(1, 2, 6);
    const right = slot(3, 2, 6);
    expect(left.x).toBe(-right.x);
    expect(left.rotate).toBe(-right.rotate);
    expect(left.scale).toBe(right.scale);
    expect(left.z).toBe(right.z);
  });

  it("tilts, sinks and shrinks cards further out, and keeps them behind", () => {
    const near = slot(3, 2, 6);
    const far = slot(4, 2, 6);
    expect(far.x).toBeGreaterThan(near.x);
    expect(far.rotate).toBeGreaterThan(near.rotate);
    expect(far.y).toBeGreaterThan(near.y);
    expect(far.scale).toBeLessThan(near.scale);
    expect(far.z).toBeLessThan(near.z);
  });

  it("hides cards beyond the reach of the fan", () => {
    expect(slot(5, 2, 6).visible).toBe(false);
    expect(slot(5, 2, 6, 3).visible).toBe(true);
  });
});
