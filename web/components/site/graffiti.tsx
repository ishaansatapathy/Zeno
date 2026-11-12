"use client";

import { m } from "motion/react";
import type { CSSProperties } from "react";

/**
 * The street kit: masking tape, a name sticker, paint splatter, drips running off a word, and a
 * marker cross-out. All decoration, all hidden from assistive technology.
 */

const ease = [0.65, 0, 0.35, 1] as const;

/** A torn strip of masking tape. Place and rotate it with `className`. */
export function Tape({ className = "", style }: { className?: string; style?: CSSProperties }) {
  return <span aria-hidden="true" style={style} className={`tape pointer-events-none absolute block ${className}`} />;
}

/** Paint flicked off a can: a lopsided blot, and droplets thrown mostly one way. */
export function Splatter({ className = "" }: { className?: string }) {
  const dots: [number, number, number][] = [
    // The blot, built from a few overlapping drops so its edge is uneven.
    [52, 58, 12], [61, 52, 9], [46, 66, 7.5], [60, 66, 6], [43, 52, 5],
    // Thrown up and to the right, getting smaller as they fly.
    [76, 44, 4.2], [84, 36, 3], [92, 30, 2.2], [99, 23, 1.5], [104, 19, 1],
    [79, 60, 3.4], [91, 58, 2.4], [101, 55, 1.6], [110, 52, 1.1],
    [71, 30, 2.4], [77, 18, 1.5], [68, 79, 2.8], [80, 88, 1.8], [88, 96, 1.2],
    // A few strays on the near side.
    [30, 48, 2.2], [24, 70, 1.6], [36, 84, 2], [18, 58, 1.1], [40, 34, 1.4],
  ];
  return (
    <svg viewBox="0 0 120 120" aria-hidden="true" focusable="false" className={`pointer-events-none ${className}`}>
      <g fill="currentColor">
        {dots.map(([x, y, r]) => (
          <circle key={`${x}-${y}`} cx={x} cy={y} r={r} />
        ))}
        <ellipse cx="74" cy="72" rx="9" ry="2.2" transform="rotate(32 74 72)" />
        <ellipse cx="70" cy="40" rx="7" ry="1.8" transform="rotate(-38 70 40)" />
      </g>
    </svg>
  );
}

/** The classic "HELLO my name is" slap sticker, with the name written in marker. */
export function HelloSticker({ name, className = "" }: { name: string; className?: string }) {
  return (
    <div aria-hidden="true" className={`pointer-events-none w-[9.5rem] rounded-[10px] bg-violet-deep p-[5px] shadow-[0_14px_30px_rgba(0,0,0,0.55)] ${className}`}>
      <p className="pt-1 text-center leading-none text-cream">
        <span className="shout block text-[1.55rem] not-italic tracking-[0.04em]">Hello</span>
        <span className="caps mt-0.5 block text-[0.62rem] tracking-[0.18em]">my name is</span>
      </p>
      <p className="mt-1.5 rounded-b-[6px] rounded-t-[2px] bg-cream px-2 pb-2 pt-1 text-center">
        <span className="tag block -rotate-[5deg] text-[2.4rem] leading-[0.9] text-ink">{name}</span>
      </p>
    </div>
  );
}

/** Paint running down from a freshly sprayed word, the drips growing after the spray lands. */
export function WordDrips({ delay = 0, className = "" }: { delay?: number; className?: string }) {
  const drops = [
    { x: 9, h: 48 },
    { x: 23, h: 22 },
    { x: 41, h: 70 },
    { x: 57, h: 30 },
    { x: 74, h: 56 },
    { x: 90, h: 18 },
  ];
  return (
    <m.svg
      viewBox="0 0 100 80"
      preserveAspectRatio="none"
      aria-hidden="true"
      focusable="false"
      className={`pointer-events-none origin-top ${className}`}
      initial={{ scaleY: 0 }}
      whileInView={{ scaleY: 1 }}
      viewport={{ once: true, margin: "0px 0px -8% 0px" }}
      transition={{ duration: 1.6, delay, ease: [0.2, 0.7, 0.3, 1] }}
    >
      <g fill="currentColor">
        {drops.map((d) => (
          <g key={d.x}>
            <rect x={d.x - 1.3} y={0} width={2.6} height={d.h} rx={1.3} />
            <ellipse cx={d.x} cy={d.h - 1.5} rx={2.1} ry={3.2} />
          </g>
        ))}
      </g>
    </m.svg>
  );
}

/** A word crossed out hard with a marker: a fat violet stroke, then a quick yellow one across it. */
export function CrossOut({ delay = 0, className = "" }: { delay?: number; className?: string }) {
  const draw = (d: number) => ({
    initial: { pathLength: 0 },
    whileInView: { pathLength: 1 },
    viewport: { once: true, margin: "0px 0px -20% 0px" },
    transition: { duration: 0.55, delay: d, ease },
  });
  return (
    <svg viewBox="0 0 400 100" preserveAspectRatio="none" aria-hidden="true" focusable="false" className={`pointer-events-none overflow-visible ${className}`}>
      <defs>
        <filter id="marker-edge" x="-5%" y="-30%" width="110%" height="160%">
          <feTurbulence type="fractalNoise" baseFrequency="0.8" numOctaves="2" seed="3" />
          <feDisplacementMap in="SourceGraphic" scale="5" />
        </filter>
      </defs>
      <m.path
        d="M4 62C80 52 170 50 250 44c50-4 100-10 146-20"
        fill="none"
        stroke="#8b5cf6"
        strokeWidth={14}
        strokeLinecap="round"
        vectorEffect="non-scaling-stroke"
        filter="url(#marker-edge)"
        {...draw(delay)}
      />
      <m.path
        d="M10 30c60 10 140 26 220 36 60 8 120 14 166 26"
        fill="none"
        stroke="#ffc21a"
        strokeWidth={6}
        strokeLinecap="round"
        vectorEffect="non-scaling-stroke"
        filter="url(#marker-edge)"
        {...draw(delay + 0.45)}
      />
    </svg>
  );
}

/** A hand-drawn ring for round buttons, a little off, the way a marker goes round. */
export function Ring({ className = "" }: { className?: string }) {
  return (
    <svg viewBox="0 0 60 60" aria-hidden="true" focusable="false" className={`pointer-events-none ${className}`}>
      <path
        d="M31 5.5c13.6.4 24 10.6 23.4 24.6-.6 13.4-11.6 24.4-25.4 24.2C15.2 54.1 5.4 43.4 5.8 29.6 6.2 16.3 16.6 6 30.6 6.4c4.2.1 7.6 1 10.4 2.6"
        fill="none"
        stroke="currentColor"
        strokeWidth={2.6}
        strokeLinecap="round"
      />
    </svg>
  );
}
