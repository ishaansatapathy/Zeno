"use client";

import { m, useMotionTemplate, useMotionValue, useSpring, useTransform, type MotionValue } from "motion/react";
import { useEffect, useRef, type RefObject } from "react";

/**
 * The hero's one object: a tile of polished chrome with a play mark cut into it. It leans a little
 * towards the pointer, and the light on its face follows the pointer too; a band of light sweeps
 * across it every few seconds. `grow` (from the scroll) scales it up just before it floods the
 * screen, and `block` is what the flood measures from.
 */
export function ChromeTile({ block, grow }: { block: RefObject<HTMLDivElement | null>; grow: MotionValue<number> }) {
  const stage = useRef<HTMLDivElement>(null);
  const px = useMotionValue(0.5);
  const py = useMotionValue(0.35);
  const spring = { stiffness: 120, damping: 18, mass: 0.6 };
  const rotateX = useSpring(useTransform(py, [0, 1], [12, -12]), spring);
  const rotateY = useSpring(useTransform(px, [0, 1], [-14, 14]), spring);
  const gx = useSpring(useTransform(px, (v) => v * 100), spring);
  const gy = useSpring(useTransform(py, (v) => v * 100), spring);
  const glare = useMotionTemplate`radial-gradient(circle at ${gx}% ${gy}%, rgba(255,255,255,0.9), rgba(255,255,255,0) 48%)`;

  useEffect(() => {
    function onMove(e: PointerEvent) {
      if (e.pointerType !== "mouse" || !stage.current) return;
      const r = stage.current.getBoundingClientRect();
      // Within a generous area around the tile, map the pointer to 0..1 on each axis.
      px.set(Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)));
      py.set(Math.min(1, Math.max(0, (e.clientY - r.top) / r.height)));
    }
    function onLeave() {
      px.set(0.5);
      py.set(0.35);
    }
    window.addEventListener("pointermove", onMove, { passive: true });
    document.documentElement.addEventListener("pointerleave", onLeave);
    return () => {
      window.removeEventListener("pointermove", onMove);
      document.documentElement.removeEventListener("pointerleave", onLeave);
    };
  }, [px, py]);

  return (
    <div ref={stage} aria-hidden="true" className="relative grid size-full place-items-center [perspective:1000px]">
      {/* A soft pool of light under the tile, and its shadow. */}
      <div className="pointer-events-none absolute left-1/2 top-1/2 h-[46%] w-[70%] -translate-x-1/2 -translate-y-1/2 rounded-full bg-[radial-gradient(closest-side,rgba(228,231,235,0.10),transparent)]" />
      <m.div animate={{ y: [0, -12, 0] }} transition={{ duration: 6, repeat: Infinity, ease: "easeInOut" }}>
        <m.div ref={block} style={{ scale: grow, rotateX, rotateY }} className="chrome relative size-[clamp(9rem,19vw,15.5rem)] overflow-hidden rounded-[22%]">
          <m.div style={{ background: glare }} className="absolute inset-0 mix-blend-soft-light" />
          <div className="sheen absolute inset-0 mix-blend-overlay" />
          <svg viewBox="0 0 100 100" focusable="false" className="absolute inset-0 size-full">
            <defs>
              <linearGradient id="engrave" x1="0" y1="0" x2="1" y2="1">
                <stop offset="0" stopColor="#0b0c0e" />
                <stop offset="1" stopColor="#2a2e34" />
              </linearGradient>
            </defs>
            {/* The cut: a light edge below and right, the dark play mark over it. */}
            <path d="M39.6 31.6v38.8L71.6 51 39.6 31.6Z" fill="rgba(255,255,255,0.85)" />
            <path d="M38 30v38.8L70 49.4 38 30Z" fill="url(#engrave)" />
          </svg>
        </m.div>
      </m.div>
      <div className="pointer-events-none absolute bottom-[14%] left-1/2 h-6 w-[38%] -translate-x-1/2 rounded-full bg-black/80 blur-xl" />
    </div>
  );
}
