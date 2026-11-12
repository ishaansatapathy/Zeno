"use client";

import { useInView, useReducedMotion } from "motion/react";
import { useEffect, useRef, useState } from "react";

import { scrambleFrame } from "@/lib/scramble";

/**
 * Text that settles out of random glyphs, once when it first comes into view and again whenever
 * `replay` changes (a hover, a new tab). Screen readers get the real text only.
 */
export function Scramble({ text, replay = 0, duration = 650, className = "" }: { text: string; replay?: number; duration?: number; className?: string }) {
  const ref = useRef<HTMLSpanElement>(null);
  const seen = useInView(ref, { once: true, margin: "0px 0px -10% 0px" });
  const reduce = useReducedMotion();
  const [shown, setShown] = useState(text);

  useEffect(() => {
    if (!seen || reduce) {
      setShown(text);
      return;
    }
    let frame = 0;
    const start = performance.now();
    const tick = (now: number) => {
      const p = (now - start) / duration;
      setShown(scrambleFrame(text, p));
      if (p < 1) frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [text, replay, seen, reduce, duration]);

  return (
    <span ref={ref} className={className}>
      <span className="sr-only">{text}</span>
      <span aria-hidden="true">{shown}</span>
    </span>
  );
}
