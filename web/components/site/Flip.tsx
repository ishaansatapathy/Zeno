"use client";

import { m } from "motion/react";
import Link from "next/link";

import { flip, links } from "@/content/site";

import { ArrowRight, ArrowUpRight } from "./frame";

const ease = [0.22, 1, 0.36, 1] as const;
const enter = (delay: number) => ({
  initial: { opacity: 0, y: 24, filter: "blur(6px)" },
  whileInView: { opacity: 1, y: 0, filter: "blur(0px)" },
  viewport: { once: true, margin: "0px 0px -15% 0px" },
  transition: { duration: 0.9, delay, ease },
});

/**
 * Where the hero's orange flood lands: the one orange screen of the page. The words arrive one
 * after another, once the colour has settled, and a ticker runs along the bottom edge.
 */
export function Flip() {
  const run = [...flip.ticker, ...flip.ticker];
  return (
    <section id="start" aria-labelledby="flip-title" className="relative -mt-px flex min-h-[100svh] flex-col bg-signal text-ink [&_*:focus-visible]:outline-ink">
      <div className="mx-auto flex w-full max-w-4xl flex-1 flex-col items-center justify-center px-5 py-28 text-center">
        <m.p {...enter(0.05)} className="font-mono text-[0.6875rem] font-medium uppercase tracking-[0.14em]">
          ● {flip.label}
        </m.p>
        <m.h2 {...enter(0.15)} id="flip-title" className="display mt-7 text-[clamp(2.6rem,6vw,5.4rem)]">
          <span className="block">{flip.title[0]}</span>
          <em className="block italic">{flip.title[1]}</em>
        </m.h2>
        <m.p {...enter(0.3)} className="mt-7 max-w-[32rem] text-[0.95rem] leading-relaxed text-ink/80">
          {flip.body}
        </m.p>
        <m.div {...enter(0.45)} className="mt-10 flex flex-wrap justify-center gap-3">
          <Link href={links.start} prefetch={false} className="caps fillup fillup-paper flex h-12 items-center gap-8 bg-ink px-6 text-signal">
            {flip.primary}
            <ArrowUpRight />
          </Link>
          <a href="#bench" className="caps fillup fillup-ink flex h-12 items-center gap-8 border border-ink px-6">
            {flip.secondary}
            <ArrowRight />
          </a>
        </m.div>
      </div>
      <div aria-hidden="true" className="overflow-hidden whitespace-nowrap border-t border-ink/20 bg-ink py-3 text-signal">
        <div className="flex w-max animate-marquee">
          {run.map((w, i) => (
            <span key={i} className="font-mono text-[0.6875rem] font-medium uppercase tracking-[0.14em]">
              <span className="px-6">{w}</span>×
            </span>
          ))}
        </div>
      </div>
    </section>
  );
}
