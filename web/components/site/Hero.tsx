"use client";

import { m, useScroll, useTransform } from "motion/react";
import { useRef } from "react";

import { hero, links } from "@/content/site";

import { Bloom } from "./Bloom";
import { Button, Container, Label } from "./frame";

/**
 * The opening: the statement, as large as the screen allows, and under a hairline the line that
 * explains it and the two ways in. The hero holds still while you scroll: the words lift away,
 * and then bright silver floods out from the middle of the screen, square by square, into the
 * section that follows.
 */
export function Hero() {
  const ref = useRef<HTMLElement>(null);
  const origin = useRef<HTMLDivElement>(null);
  const { scrollYProgress } = useScroll({ target: ref, offset: ["start start", "end end"] });

  const wordsOpacity = useTransform(scrollYProgress, [0.04, 0.28], [1, 0]);
  const wordsY = useTransform(scrollYProgress, [0, 0.28], [0, -60]);
  const hint = useTransform(scrollYProgress, [0, 0.06], [1, 0]);
  const bloom = useTransform(scrollYProgress, [0.3, 0.74], [0, 1]);

  return (
    <section ref={ref} aria-labelledby="hero-title" className="relative h-[190svh]">
      <div className="sticky top-0 h-[100svh] overflow-hidden">
        <Container className="flex h-full flex-col justify-center pb-14 pt-24 lg:pb-16 lg:pt-28">
          <m.div style={{ opacity: wordsOpacity, y: wordsY }} className="relative z-10">
            <div className="rise">
              <Label>Video courses · live classes</Label>
            </div>
            <h1 id="hero-title" className="display mt-6 text-[clamp(3.4rem,min(12.4vw,20svh),13rem)] leading-[0.94] lg:mt-8">
              <span className="rise block" style={{ ["--delay" as string]: "0.06s" }}>
                {hero.title[0]}
              </span>
              <span className="rise block" style={{ ["--delay" as string]: "0.16s" }}>
                <em className="metal">{hero.title[1]}</em>
              </span>
            </h1>
            <div className="rise mt-10 grid gap-8 border-t border-line pt-8 lg:mt-14 lg:grid-cols-[minmax(0,1fr)_auto] lg:items-end" style={{ ["--delay" as string]: "0.3s" }}>
              <p className="max-w-[36rem] text-[clamp(1rem,1.25vw,1.2rem)] leading-relaxed text-mute">
                {hero.lines[0]} {hero.lines[1]} <span className="text-silver">{hero.accentLine}</span>
              </p>
              <div className="flex flex-wrap gap-3">
                <Button href={links.start} out>
                  Start teaching
                </Button>
                <Button href="#bench" tone="line">
                  See how it works
                </Button>
              </div>
            </div>
          </m.div>
        </Container>

        {/* Where the flood starts: the middle of the screen. */}
        <div ref={origin} aria-hidden="true" className="pointer-events-none absolute left-1/2 top-1/2 size-px" />
        <m.p style={{ opacity: hint }} className="label absolute bottom-5 left-1/2 z-10 -translate-x-1/2">
          Scroll ↓
        </m.p>
        <Bloom progress={bloom} origin={origin} color="#e7eaee" />
      </div>
    </section>
  );
}
