"use client";

import { AnimatePresence, m, useReducedMotion } from "motion/react";
import { useId, useState } from "react";

import { cn } from "@/components/ui/cn";

import { ZenoTile } from "./ZenoTile";

type Node = { key: string; title: string; hint: string; x: number; y: number };

/** The loop the money and the lessons travel, through the four nodes, in the square's own units. */
const LOOP = "M14 22 C 38 4, 62 4, 86 22 C 100 40, 100 60, 86 78 C 62 96, 38 96, 14 78 C 0 60, 0 40, 14 22 Z";
const PLACES = [
  { x: 14, y: 22 },
  { x: 86, y: 22 },
  { x: 86, y: 78 },
  { x: 14, y: 78 },
];

/**
 * Zeno in one picture: the tile at the centre, and around it the four things it does for a
 * lesson, joined in a loop that small saffron pulses keep travelling. Each node can be hovered,
 * focused or tapped to say what it means, and its spoke to the centre lights up while it does.
 */
export function HeroCore({ label, nodes }: { label: string; nodes: { key: string; title: string; hint: string }[] }) {
  const reduce = useReducedMotion();
  const base = useId();
  const [active, setActive] = useState<string | null>(null);
  const placed: Node[] = nodes.map((node, i) => ({ ...node, ...PLACES[i] }));

  return (
    <figure aria-label={label} className="relative mx-auto aspect-square w-full max-w-[34rem]">
      <svg viewBox="0 0 100 100" className="absolute inset-0 h-full w-full overflow-visible" aria-hidden="true">
        <m.path
          d={LOOP}
          fill="none"
          stroke="rgb(var(--text))"
          strokeWidth="0.35"
          strokeDasharray="1.2 1.4"
          initial={{ pathLength: 0, opacity: 0 }}
          animate={{ pathLength: 1, opacity: 1 }}
          transition={{ duration: 1.8, delay: 0.5, ease: [0.65, 0, 0.35, 1] }}
        />
        {placed.map((node) => (
          <m.line
            key={node.key}
            x1={node.x}
            y1={node.y}
            x2={50}
            y2={50}
            stroke="rgb(var(--text))"
            strokeLinecap="round"
            animate={{ strokeWidth: active === node.key ? 0.55 : 0.22, opacity: active === node.key ? 1 : 0.35 }}
            transition={{ duration: 0.3 }}
          />
        ))}
        {!reduce &&
          [0, 1, 2].map((i) => (
            <circle key={i} r="1.25" fill="rgb(var(--accent))" stroke="rgb(var(--text))" strokeWidth="0.3">
              <animateMotion dur="9s" repeatCount="indefinite" begin={`${-i * 3}s`} path={LOOP} />
            </circle>
          ))}
      </svg>

      <div className="absolute inset-[20%]">
        <ZenoTile />
      </div>

      {placed.map((node, i) => {
        const hintId = `${base}-${node.key}`;
        const on = active === node.key;
        const below = node.y < 50;
        return (
          <div key={node.key} className="absolute" style={{ left: `${node.x}%`, top: `${node.y}%` }}>
            <m.button
              type="button"
              aria-describedby={on ? hintId : undefined}
              aria-expanded={on}
              onPointerEnter={() => setActive(node.key)}
              onPointerLeave={() => setActive(null)}
              onFocus={() => setActive(node.key)}
              onBlur={() => setActive(null)}
              onClick={() => setActive(on ? null : node.key)}
              initial={{ opacity: 0, scale: 0.6 }}
              animate={{ opacity: 1, scale: 1 }}
              transition={{ type: "spring", stiffness: 260, damping: 18, delay: 0.9 + i * 0.12 }}
              className={cn(
                "flex -translate-x-1/2 -translate-y-1/2 items-center gap-2 whitespace-nowrap rounded-full border border-text px-3 py-1.5 font-mono text-[0.68rem] uppercase tracking-label shadow-[2px_2px_0_0_rgb(var(--text))] transition-colors sm:text-[0.72rem]",
                on ? "bg-accent text-on-accent" : "bg-surface text-text",
              )}
            >
              <span className="text-text-muted">{String(i + 1).padStart(2, "0")}</span>
              {node.title}
            </m.button>
            <AnimatePresence>
              {on && (
                <m.p
                  id={hintId}
                  role="tooltip"
                  initial={{ opacity: 0, y: below ? -4 : 4 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0 }}
                  transition={{ duration: 0.18 }}
                  className={cn(
                    "absolute z-20 w-52 rounded-control border border-text bg-surface p-3 text-[0.82rem] leading-snug text-text shadow-[3px_3px_0_0_rgb(var(--text))]",
                    below ? "top-6" : "bottom-6",
                    node.x < 50 ? "left-[-1rem]" : "right-[-1rem]",
                  )}
                >
                  {node.hint}
                </m.p>
              )}
            </AnimatePresence>
          </div>
        );
      })}
    </figure>
  );
}
