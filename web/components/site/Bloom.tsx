"use client";

import { useMotionValueEvent, type MotionValue } from "motion/react";
import { useEffect, useRef, type RefObject } from "react";

import { bloomThresholds } from "@/lib/bloom";

/**
 * Floods the screen with `color`, square by square, from the hero's block outwards, as `progress`
 * goes from 0 to 1. At 1 every cell is filled, and the section that follows the hero, in the same
 * colour, takes over without a seam.
 */
export function Bloom({ progress, origin, color }: { progress: MotionValue<number>; origin: RefObject<HTMLDivElement | null>; color: string }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const grid = useRef<{ cols: number; rows: number; size: number; thresholds: number[] } | null>(null);

  function layout() {
    const canvas = canvasRef.current;
    const block = origin.current;
    if (!canvas || !block) return;
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const w = canvas.clientWidth;
    const h = canvas.clientHeight;
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
    canvas.getContext("2d")?.setTransform(dpr, 0, 0, dpr, 0, 0);
    const size = w < 640 ? 30 : 46;
    const cols = Math.ceil(w / size);
    const rows = Math.ceil(h / size);
    const c = canvas.getBoundingClientRect();
    const b = block.getBoundingClientRect();
    const cx = (b.left + b.width / 2 - c.left) / size;
    const cy = (b.top + b.height / 2 - c.top) / size;
    grid.current = { cols, rows, size, thresholds: bloomThresholds(cols, rows, cx, cy) };
  }

  function paint(p: number) {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext("2d");
    const g = grid.current;
    if (!canvas || !ctx || !g) return;
    ctx.clearRect(0, 0, canvas.clientWidth, canvas.clientHeight);
    if (p <= 0) return;
    ctx.fillStyle = color;
    // Cells overlap by half a pixel so no hairline of black shows between them.
    for (let r = 0; r < g.rows; r++) {
      for (let c = 0; c < g.cols; c++) {
        if (g.thresholds[r * g.cols + c] <= p) ctx.fillRect(c * g.size - 0.5, r * g.size - 0.5, g.size + 1, g.size + 1);
      }
    }
  }

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ro = new ResizeObserver(() => {
      layout();
      paint(progress.get());
    });
    ro.observe(canvas);
    return () => ro.disconnect();
  }, []);

  // The block grows from its own centre, so the centre measured on layout stays true.
  useMotionValueEvent(progress, "change", paint);

  return <canvas ref={canvasRef} aria-hidden="true" className="pointer-events-none absolute inset-0 z-20 size-full" />;
}
