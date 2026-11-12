"use client";

import type { MotionValue } from "motion/react";
import { useEffect, useRef, type RefObject } from "react";

import { GLYPHS } from "@/lib/scramble";

/** Raw video, as characters: light to dense. */
const RAW = " .,:;-=+*#%@";
/** Locked segments: plainer, blockier. */
const LOCKED = " .-=+#";
const SIGNAL = "#ff5b14";
const PAPER = "242, 239, 233";
/** Columns per locked segment, plus one blank column between segments. */
const SEG = 6;

/**
 * The hero's picture, drawn in monospaced characters on a canvas. From the left, a ribbon of raw
 * video flows in; it narrows into the orange block in the middle (the platform) and leaves on the
 * right cut into even, locked segments, one of them flashing orange as it is sealed. The pointer
 * stirs the characters near it. `pull` (0..1, from the scroll) draws the ribbons in tight and
 * speeds them up as the hero hands over to the next section.
 *
 * Runs at 30 frames a second, only while on screen; with reduced motion it draws one still frame.
 */
export function Stream({ pull, block }: { pull: MotionValue<number>; block: RefObject<HTMLDivElement | null> }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx) return;

    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const family = getComputedStyle(document.documentElement).getPropertyValue("--font-mono").trim() || "monospace";
    let w = 0;
    let h = 0;
    let fs = 12;
    let cw = 7;
    let lh = 14;
    let visible = true;
    let raf = 0;
    let last = 0;
    const born = performance.now();
    const pointer = { x: -1e4, y: -1e4 };

    function resize() {
      if (!canvas || !ctx) return;
      const dpr = Math.min(window.devicePixelRatio || 1, 2);
      w = canvas.clientWidth;
      h = canvas.clientHeight;
      canvas.width = Math.round(w * dpr);
      canvas.height = Math.round(h * dpr);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      fs = w < 640 ? 10 : 12;
      ctx.font = `${fs}px ${family}`;
      cw = ctx.measureText("M").width || fs * 0.6;
      lh = Math.round(fs * 1.18);
      ctx.textBaseline = "top";
    }

    function draw(now: number) {
      if (!ctx) return;
      const t = (now - born) / 1000;
      const intro = reduce ? 1 : Math.min(1, t / 1.8);
      const eased = 1 - Math.pow(1 - intro, 3);
      const p = pull.get();
      ctx.clearRect(0, 0, w, h);
      ctx.font = `${fs}px ${family}`;

      const cols = Math.floor(w / cw);
      const rows = Math.floor(h / lh);
      const cx = cols / 2;
      const midRow = rows / 2;
      // Keep clear of the orange block, whatever size it is right now.
      const rect = block.current?.getBoundingClientRect();
      const box = rect ? rect.width : 64;
      const gap = box / 2 / cw + 2;
      const half = Math.max(0.08, box / 2 / (h / 2));
      const front = (t * 1.6) % 14;
      const flash: [number, number, string][] = [];

      for (let r = 0; r < rows; r++) {
        const v = (r + 0.5 - midRow) / midRow;
        const y = r * lh;
        for (let c = 0; c < cols; c++) {
          const du = c + 0.5 - cx;
          const a = Math.abs(du);
          if (a < gap) continue;
          const au = Math.min(1, a / cx);
          // The ribbons arrive from the edges first.
          if (au < 1 - eased) continue;
          const side = du < 0 ? -1 : 1;
          // Three strands per side, braided: each its own wave, all narrowing into the block.
          let base = 0;
          for (let k = 0; k < 3; k++) {
            const amp = (0.16 + 0.13 * k) * Math.pow(au, 1.05) * (1 - p * 0.8);
            const centre = Math.sin(au * (3.1 + k * 1.4) - t * (0.55 + 0.22 * k) * side + k * 2.1 + side) * amp;
            const thick = half * 0.8 + (0.34 - half * 0.4) * Math.pow(au, 1.2) * (1 - p * 0.5);
            const d = Math.abs(v - centre) / thick;
            if (d < 1) base = Math.max(base, Math.pow(1 - d, 1.35));
          }
          if (base <= 0) continue;
          const x = c * cw;

          let ch: string;
          let alpha: number;
          let orange = false;
          if (side < 0) {
            const speed = 5 + p * 14;
            const tex = 0.5 + 0.5 * Math.sin(c * 0.55 - t * speed + r * 0.9) * Math.cos(r * 0.37 + c * 0.11 + t);
            const i = base * (0.3 + 0.7 * tex);
            ch = RAW[Math.min(RAW.length - 1, Math.floor(i * RAW.length))];
            alpha = 0.16 + 0.84 * i;
          } else {
            const along = a - gap;
            if (along % (SEG + 1) >= SEG) continue;
            const seg = Math.floor(along / (SEG + 1));
            ch = LOCKED[Math.min(LOCKED.length - 1, Math.floor(base * LOCKED.length))];
            alpha = 0.18 + 0.7 * base;
            if (seg === Math.floor(front) && base > 0.15) orange = true;
          }
          if (ch === " ") continue;

          // The pointer stirs whatever is near it.
          const dx = x - pointer.x;
          const dy = y - pointer.y;
          const near = dx * dx + dy * dy;
          if (near < 90 * 90) {
            ch = GLYPHS[Math.floor(Math.random() * GLYPHS.length)];
            alpha = Math.min(1, alpha + 0.45);
            if (near < 38 * 38) orange = true;
          }

          if (orange) {
            flash.push([x, y, ch]);
            continue;
          }
          ctx.fillStyle = `rgba(${PAPER}, ${alpha.toFixed(2)})`;
          ctx.fillText(ch, x, y);
        }
      }
      ctx.fillStyle = SIGNAL;
      for (const [x, y, ch] of flash) ctx.fillText(ch, x, y);
    }

    function loop(now: number) {
      raf = requestAnimationFrame(loop);
      if (!visible || document.hidden) return;
      if (now - last < 1000 / 30) return;
      last = now;
      draw(now);
    }

    function onPointer(e: PointerEvent) {
      if (!canvas || e.pointerType !== "mouse") return;
      const r = canvas.getBoundingClientRect();
      pointer.x = e.clientX - r.left;
      pointer.y = e.clientY - r.top;
    }
    function onLeave() {
      pointer.x = -1e4;
      pointer.y = -1e4;
    }

    const ro = new ResizeObserver(() => {
      resize();
      if (reduce) draw(performance.now());
    });
    const io = new IntersectionObserver(([entry]) => {
      visible = entry.isIntersecting;
    });

    document.fonts.ready.then(() => {
      resize();
      ro.observe(canvas);
      io.observe(canvas);
      if (reduce) draw(performance.now() + 1e5);
      else raf = requestAnimationFrame(loop);
    });
    window.addEventListener("pointermove", onPointer, { passive: true });
    document.documentElement.addEventListener("pointerleave", onLeave);

    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
      io.disconnect();
      window.removeEventListener("pointermove", onPointer);
      document.documentElement.removeEventListener("pointerleave", onLeave);
    };
  }, [pull, block]);

  return <canvas ref={canvasRef} aria-hidden="true" className="absolute inset-0 size-full" />;
}
