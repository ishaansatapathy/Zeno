"use client";

import { m, useScroll, useTransform } from "motion/react";
import { useRef } from "react";

/** The ink line comes in from the left as the page arrives here and ties a loose loop. */
export function CloseLine() {
  const ref = useRef<HTMLDivElement>(null);
  const { scrollYProgress } = useScroll({ target: ref, offset: ["start end", "center center"] });
  const draw = useTransform(scrollYProgress, [0, 1], [0, 1]);
  return (
    <div ref={ref} className="pointer-events-none absolute inset-x-0 top-6 h-20 sm:top-8 sm:h-40">
    <svg viewBox="0 0 1200 200" preserveAspectRatio="none" className="h-full w-full" aria-hidden="true">
      <m.path
        d="M-10 120 C 180 120, 300 40, 470 70 C 610 95, 560 170, 500 150 C 440 130, 520 40, 650 60 C 820 86, 980 120, 1210 90"
        fill="none"
        stroke="rgb(var(--text))"
        strokeWidth="1.6"
        strokeLinecap="round"
        vectorEffect="non-scaling-stroke"
        style={{ pathLength: draw }}
      />
    </svg>
    </div>
  );
}
