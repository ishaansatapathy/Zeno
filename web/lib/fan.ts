/**
 * Where each card of the deck sits when one of them is in front: the active card upright and
 * raised in the middle, the others fanned out to either side, tilting further and sinking lower
 * the further they are from it. The deck wraps, so there is always a neighbour on both sides.
 */

export type Slot = {
  /** Signed distance from the active card, after wrapping: 0 is the active card. */
  offset: number;
  /** Horizontal shift, as a fraction of one card's width. */
  x: number;
  /** Vertical drop, as a fraction of one card's height. */
  y: number;
  rotate: number;
  scale: number;
  z: number;
  /** Cards beyond the fan are hidden rather than piled up. */
  visible: boolean;
};

/** The signed shortest distance from `active` to `index` around a ring of `count` cards. */
export function ringOffset(index: number, active: number, count: number): number {
  if (count <= 0) return 0;
  let d = (((index - active) % count) + count) % count;
  if (d > count / 2) d -= count;
  // An even deck has one card exactly opposite; put it on the right, consistently.
  if (count % 2 === 0 && d === -count / 2) d = count / 2;
  return d;
}

/** Normalises any integer (after many clicks either way) into 0..count-1. */
export function wrap(index: number, count: number): number {
  return ((index % count) + count) % count;
}

export function slot(index: number, active: number, count: number, reach = 2): Slot {
  const offset = ringOffset(index, active, count);
  const a = Math.abs(offset);
  const side = Math.sign(offset);
  return {
    offset,
    x: side * (a === 0 ? 0 : 0.62 + (a - 1) * 0.5),
    y: a * 0.035 + (a > 1 ? 0.02 : 0),
    rotate: side * (a === 0 ? 0 : 7 + (a - 1) * 4),
    scale: a === 0 ? 1 : Math.max(0.7, 0.88 - (a - 1) * 0.08),
    z: 10 - a,
    visible: a <= reach,
  };
}
