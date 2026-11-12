"use client";

import { m } from "motion/react";

const ease = [0.65, 0, 0.35, 1] as const;

/**
 * A word sprayed into a headline in marker, revealed left to right the first time it is seen,
 * the way a can moves across a wall. With `underline`, a quick scribble runs under it after.
 */
export function Spray({
  children,
  delay = 0,
  underline = false,
  className = "",
  tone = "text-spray",
}: {
  children: string;
  delay?: number;
  underline?: boolean;
  className?: string;
  tone?: string;
}) {
  return (
    <span className={`relative inline-block align-baseline ${className}`}>
      <m.span
        className={`tag relative inline-block -rotate-[4deg] text-[1.3em] leading-[0.7] ${tone}`}
        initial={{ clipPath: "inset(-40% 100% -40% -10%)" }}
        whileInView={{ clipPath: "inset(-40% -10% -40% -10%)" }}
        viewport={{ once: true, margin: "0px 0px -8% 0px" }}
        transition={{ duration: 0.85, delay, ease }}
      >
        {children}
      </m.span>
      {underline ? (
        <svg
          viewBox="0 0 240 16"
          preserveAspectRatio="none"
          aria-hidden="true"
          focusable="false"
          className={`absolute -bottom-[0.3em] left-[2%] h-[0.16em] w-[98%] -rotate-[3deg] ${tone}`}
        >
          <m.path
            d="M3 10.5C46 4.2 96 3.8 140 6.4c34 2 64 1.4 97-3.6"
            fill="none"
            stroke="currentColor"
            strokeWidth={5}
            strokeLinecap="round"
            initial={{ pathLength: 0 }}
            whileInView={{ pathLength: 1 }}
            viewport={{ once: true, margin: "0px 0px -8% 0px" }}
            transition={{ duration: 0.6, delay: delay + 0.75, ease }}
          />
        </svg>
      ) : null}
    </span>
  );
}
