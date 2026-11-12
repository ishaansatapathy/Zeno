/**
 * The orange bloom that floods the screen at the end of the hero. The screen is cut into square
 * cells; each gets a threshold between 0 and 1, and a cell turns orange once the scroll progress
 * passes it. Thresholds grow with distance from the centre (a blend of square and diamond
 * distance, so it opens as a chunky cross rather than a circle), with a little fixed jitter so
 * the edge looks pixel-built instead of ruled.
 */

/** A deterministic 0..1 value per cell, so the bloom looks the same on every visit. */
function jitter(col: number, row: number): number {
  const x = Math.sin(col * 127.1 + row * 311.7) * 43758.5453;
  return x - Math.floor(x);
}

export function bloomThresholds(cols: number, rows: number, cx: number, cy: number): number[] {
  const out: number[] = new Array(cols * rows);
  let max = 0;
  for (let r = 0; r < rows; r++) {
    for (let c = 0; c < cols; c++) {
      const dx = Math.abs(c + 0.5 - cx);
      const dy = Math.abs(r + 0.5 - cy);
      const d = 0.55 * Math.max(dx, dy) + 0.45 * (dx + dy) * 0.62 + jitter(c, r) * 0.9;
      out[r * cols + c] = d;
      if (d > max) max = d;
    }
  }
  return out.map((d) => (max === 0 ? 0 : d / max));
}
