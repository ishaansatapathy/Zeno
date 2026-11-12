"use client";

import { m, useScroll, useTransform, type MotionValue } from "motion/react";
import { useRef } from "react";

import { statement } from "@/content/site";

import { Crown } from "./doodles";
import { DripButton } from "./DripButton";

/** One word that lights up as the reading line passes it. */
function Word({ word, progress, range, className = "" }: { word: string; progress: MotionValue<number>; range: [number, number]; className?: string }) {
  const opacity = useTransform(progress, range, [0.42, 1]);
  return (
    <m.span style={{ opacity }} className={className}>
      {word}{" "}
    </m.span>
  );
}

export function Statement() {
  const ref = useRef<HTMLParagraphElement>(null);
  const { scrollYProgress } = useScroll({ target: ref, offset: ["start 0.85", "end 0.5"] });
  const words = statement.text.split(" ");
  const last = statement.last.split(" ");
  const total = words.length + last.length;
  const range = (i: number): [number, number] => [i / total, Math.min(1, (i + 1.5) / total)];

  return (
    <section aria-label="What Zeno does" className="relative mx-auto max-w-page px-4 py-28 sm:px-6 lg:px-8 lg:py-40">
      <Crown className="absolute right-6 top-14 w-14 rotate-[10deg] text-cream/50 lg:right-16 lg:w-20" />
      <div className="grid gap-12 lg:grid-cols-[minmax(0,1fr)_17rem] lg:items-end lg:gap-20">
        <div>
          <p ref={ref} className="shout max-w-[56rem] text-[clamp(1.85rem,3.6vw,3.4rem)] leading-[0.98]">
            {words.map((w, i) => (
              <Word key={`${w}-${i}`} word={w} progress={scrollYProgress} range={range(i)} />
            ))}
            {last.map((w, i) => (
              <Word key={`last-${w}-${i}`} word={w} progress={scrollYProgress} range={range(words.length + i)} className="text-violet-hi" />
            ))}
          </p>
          <DripButton href={statement.cta.href} tone="smoke" drips={false} className="mt-12">
            {statement.cta.label}
          </DripButton>
        </div>
        <div className="lg:pb-2">
          <p className="text-[0.95rem] leading-relaxed text-dust">{statement.aside}</p>
          <ul className="mt-6 flex flex-wrap gap-2">
            {statement.chips.map((c) => (
              <li key={c} className="caps rounded-[4px] border border-cream/35 px-2.5 py-1.5 text-[0.72rem] text-cream">
                {c}
              </li>
            ))}
          </ul>
        </div>
      </div>
    </section>
  );
}
