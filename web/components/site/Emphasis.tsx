"use client";

import { m } from "motion/react";
import type { ReactNode } from "react";

/**
 * The one word that matters in a headline: set in the serif's italic, with a saffron stroke drawn
 * under it by hand (a slightly uneven path, not a straight rule) the first time it is seen.
 */
export function Emphasis({ children }: { children: ReactNode }) {
  return (
    <em className="relative inline-block whitespace-nowrap font-serif italic">
      <span className="relative z-10">{children}</span>
      <svg
        aria-hidden="true"
        viewBox="0 0 300 24"
        preserveAspectRatio="none"
        className="absolute -bottom-[0.08em] left-[-2%] z-0 h-[0.32em] w-[104%] text-accent"
      >
        <m.path
          d="M3 15 C 52 7, 104 18, 152 11 S 250 6, 297 13"
          fill="none"
          stroke="currentColor"
          strokeWidth="9"
          strokeLinecap="round"
          initial={{ pathLength: 0 }}
          whileInView={{ pathLength: 1 }}
          viewport={{ once: true, margin: "0px 0px -10% 0px" }}
          transition={{ duration: 0.9, delay: 0.35, ease: [0.65, 0, 0.35, 1] }}
        />
      </svg>
    </em>
  );
}
