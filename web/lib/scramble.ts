/**
 * Text that settles out of noise: characters lock into place from left to right as `progress`
 * goes from 0 to 1, and everything not yet locked shows a random glyph. Spaces stay spaces, so
 * the shape of the words is there from the first frame.
 */

export const GLYPHS = "ABCDEFGHJKLMNPQRSTUVWXYZ0123456789#$%&*+=<>/";

export function scrambleFrame(target: string, progress: number, random: () => number = Math.random): string {
  const p = Math.min(1, Math.max(0, progress));
  const locked = Math.floor(p * target.length);
  let out = "";
  for (let i = 0; i < target.length; i++) {
    const ch = target[i];
    if (i < locked || ch === " " || p >= 1) out += ch;
    else out += GLYPHS[Math.floor(random() * GLYPHS.length) % GLYPHS.length];
  }
  return out;
}
