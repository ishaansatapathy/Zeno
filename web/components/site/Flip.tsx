"use client";

import { m } from "motion/react";

import { flip, links } from "@/content/site";

import { Button } from "./frame";

const ease = [0.22, 1, 0.36, 1] as const;
const enter = (delay: number) => ({
  initial: { opacity: 0, y: 22, filter: "blur(6px)" },
  whileInView: { opacity: 1, y: 0, filter: "blur(0px)" },
  viewport: { once: true, margin: "0px 0px -5% 0px" },
  transition: { duration: 1, delay, ease },
});

/**
 * Where the hero's silver flood lands: the page's one light screen, a soft silver with a faint dot
 * grid of its own. Once the colour has settled the words arrive one after another; a ticker runs
 * along the foot.
 */
export function Flip() {
  const run = [...flip.ticker, ...flip.ticker];
  return (
    <section
      id="start"
      aria-labelledby="flip-title"
      className="relative -mt-px bg-chrome text-ink [&_*:focus-visible]:outline-ink"
      style={{ backgroundImage: "radial-gradient(rgba(0,0,0,0.18) 1px, transparent 1.3px)", backgroundSize: "32px 32px" }}
    >
      <div className="mx-auto flex min-h-[88svh] max-w-4xl flex-col items-center px-5 pb-28 pt-[16svh] text-center">
        <m.p {...enter(0.05)} className="flex items-center gap-2.5 font-mono text-[0.6875rem] font-medium uppercase tracking-[0.14em] text-chrome-mute">
          <span aria-hidden="true" className="size-1.5 bg-current" />
          {flip.label}
        </m.p>
        <m.h2 {...enter(0.15)} id="flip-title" className="display mt-7 text-[clamp(2.9rem,6.8vw,6.4rem)]">
          <span className="block">{flip.title[0]}</span>
          <em className="block pr-[0.08em] italic">{flip.title[1]}</em>
        </m.h2>
        <m.p {...enter(0.28)} className="mt-7 max-w-[34rem] text-[1rem] leading-relaxed text-chrome-mute">
          {flip.body}
        </m.p>
        <m.div {...enter(0.4)} className="mt-10 flex flex-wrap justify-center gap-3">
          <Button href={links.start} tone="ink" out>
            {flip.primary}
          </Button>
          <Button href="#bench" tone="line-ink">
            {flip.secondary}
          </Button>
        </m.div>
      </div>
      <div aria-hidden="true" className="overflow-hidden whitespace-nowrap bg-ink py-3">
        <div className="flex w-max animate-marquee">
          {run.map((w, i) => (
            <span key={i} className="flex items-center font-mono text-[0.6875rem] font-medium uppercase tracking-[0.14em] text-silver">
              <span className="px-6">{w}</span>×
            </span>
          ))}
        </div>
      </div>
    </section>
  );
}
