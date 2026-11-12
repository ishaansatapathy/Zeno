"use client";

import { m } from "motion/react";

import { flip, links } from "@/content/site";

import { Button, Container, Label, Title } from "./frame";

const ease = [0.22, 1, 0.36, 1] as const;
const enter = (delay: number) => ({
  initial: { opacity: 0, y: 20, filter: "blur(6px)" },
  whileInView: { opacity: 1, y: 0, filter: "blur(0px)" },
  viewport: { once: true, margin: "0px 0px -12% 0px" },
  transition: { duration: 1, delay, ease },
});

/**
 * The page's one light section, where the hero's panel opens out to. Dark ink on bone; the words
 * arrive one after another once the light has settled, and a quiet ticker runs along the foot.
 */
export function Flip() {
  const run = [...flip.ticker, ...flip.ticker];
  return (
    <section id="start" aria-labelledby="flip-title" className="relative -mt-px bg-bone text-ink [&_*:focus-visible]:outline-ink">
      <Container className="flex min-h-[100svh] flex-col justify-center py-28">
        <m.div {...enter(0.05)}>
          <Label n="01" className="text-bone-mute">
            {flip.label}
          </Label>
        </m.div>
        <m.div {...enter(0.15)}>
          <Title id="flip-title" tone="light" text={flip.title} className="mt-7 text-[clamp(2.6rem,6.2vw,5.8rem)]" />
        </m.div>
        <div className="mt-12 grid gap-8 border-t border-bone-line pt-8 lg:grid-cols-[minmax(0,1fr)_auto] lg:items-end">
          <m.p {...enter(0.28)} className="max-w-[34rem] text-[1rem] leading-relaxed text-bone-mute">
            {flip.body}
          </m.p>
          <m.div {...enter(0.36)} className="flex flex-wrap gap-3">
            <Button href={links.start} tone="dark">
              {flip.primary}
            </Button>
            <Button href="#bench" tone="line-dark">
              {flip.secondary}
            </Button>
          </m.div>
        </div>
      </Container>
      <div aria-hidden="true" className="overflow-hidden whitespace-nowrap border-t border-bone-line py-4">
        <div className="flex w-max animate-marquee">
          {run.map((w, i) => (
            <span key={i} className="flex items-center font-mono text-[0.7rem] uppercase tracking-[0.12em] text-bone-mute">
              <span className="px-8">{w}</span>
              <span className="size-1 rounded-full bg-bone-mute/50" />
            </span>
          ))}
        </div>
      </div>
    </section>
  );
}
