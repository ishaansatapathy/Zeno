// The toolbar icons: the silver tile with a play mark, as the site's wordmark has it, drawn with no
// image tools so the build needs nothing but Node.

import { crc32, deflateSync } from "node:zlib";

function chunk(type, data) {
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body) >>> 0);
  return Buffer.concat([length, body, crc]);
}

/** A square PNG: silver, with a black triangle pointing right. Four samples a pixel, so its edges are smooth. */
export function icon(size) {
  const silver = [0xe4, 0xe7, 0xeb];
  // The triangle from the 20x20 wordmark, in its coordinates: (7.5, 5.5), (7.5, 14.5), (14.5, 10).
  const triangle = [
    [7.5, 5.5],
    [7.5, 14.5],
    [14.5, 10],
  ].map(([x, y]) => [(x / 20) * size, (y / 20) * size]);
  const inside = (px, py) => {
    const [[ax, ay], [bx, by], [cx, cy]] = triangle;
    const side = (x1, y1, x2, y2) => (px - x2) * (y1 - y2) - (x1 - x2) * (py - y2);
    const d1 = side(ax, ay, bx, by);
    const d2 = side(bx, by, cx, cy);
    const d3 = side(cx, cy, ax, ay);
    return !((d1 < 0 || d2 < 0 || d3 < 0) && (d1 > 0 || d2 > 0 || d3 > 0));
  };
  const samples = [
    [0.25, 0.25],
    [0.75, 0.25],
    [0.25, 0.75],
    [0.75, 0.75],
  ];
  const rows = [];
  for (let y = 0; y < size; y += 1) {
    const row = Buffer.alloc(1 + size * 4);
    for (let x = 0; x < size; x += 1) {
      let covered = 0;
      for (const [dx, dy] of samples) if (inside(x + dx, y + dy)) covered += 1;
      const k = covered / 4;
      for (let c = 0; c < 3; c += 1) row[1 + x * 4 + c] = Math.round(silver[c] * (1 - k));
      row[1 + x * 4 + 3] = 255;
    }
    rows.push(row);
  }
  const header = Buffer.alloc(13);
  header.writeUInt32BE(size, 0);
  header.writeUInt32BE(size, 4);
  header[8] = 8; // bits per channel
  header[9] = 6; // RGBA
  return Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk("IHDR", header), chunk("IDAT", deflateSync(Buffer.concat(rows))), chunk("IEND", Buffer.alloc(0))]);
}
