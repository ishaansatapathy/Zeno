"use client";

import { m, useScroll, useTransform } from "motion/react";
import { useRef } from "react";

import { hero, links } from "@/content/site";

import { Bloom } from "./Bloom";
import { ChromeTile } from "./ChromeTile";
import { Button, Container, Label } from "./frame";

/**
 * The opening: the statement on the left, and on the right one tile of polished chrome with a play
 * mark cut into it. The hero holds still while you scroll: the words lift away, the tile grows,
 * and then bright silver floods out from it, square by square, into the section that follows.
 */
export function Hero() {
  const ref = useRef<HTMLElement>(null);
  const block = useRef<HTMLDivElement>(null);
  const { scrollYProgress } = useScroll({ target: ref, offset: ["start start", "end end"] });

  const wordsOpacity = useTransform(scrollYProgress, [0.04, 0.28], [1, 0]);
  const wordsY = useTransform(scrollYProgress, [0, 0.28], [0, -60]);
  const hint = useTransform(scrollYProgress, [0, 0.06], [1, 0]);
    const grow = useTransform(scrollYProgress, [0.08, 0.36], [1, 1.35]);
  const bloom = useTransform(scrollYProgress, [0.3, 0.74], [0, 1]);

  return (
    <section ref={ref} aria-labelledby="hero-title" className="relative h-[190svh]">
      <div className="sticky top-0 h-[100svh] overflow-hidden">
        <Container className="grid h-full grid-rows-[auto_minmax(0,1fr)] gap-6 pb-8 pt-[clamp(5.5rem,13svh,8rem)] lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)] lg:grid-rows-1 lg:items-center lg:gap-6 lg:pb-8 lg:pt-16">
          <m.div style={{ opacity: wordsOpacity, y: wordsY }} className="relative z-10">
            <div className="rise">
              <Label>Video courses · live classes</Label>
            </div>
            <h1 id="hero-title" className="display mt-6 text-[clamp(2.9rem,min(6.4vw,11svh),6.8rem)] lg:mt-8">
              <span className="rise block" style={{ ["--delay" as string]: "0.06s" }}>
                {hero.title[0]}
              </span>
              <span className="rise block" style={{ ["--delay" as string]: "0.16s" }}>
                <em className="metal">{hero.title[1]}</em>
              </span>
            </h1>
            <p className="rise mt-6 max-w-[31rem] text-[0.98rem] leading-relaxed text-mute lg:mt-8" style={{ ["--delay" as string]: "0.28s" }}>
              {hero.lines[0]} {hero.lines[1]} <span className="text-silver">{hero.accentLine}</span>
            </p>
            <div className="rise mt-8 flex flex-wrap gap-3 lg:mt-10" style={{ ["--delay" as string]: "0.38s" }}>
              <Button href={links.start} out>
                Start teaching
              </Button>
              <Button href="#bench" tone="line">
                See how it works
              </Button>
            </div>
          </m.div>

          <div className="relative min-h-[18rem] lg:h-[min(78svh,46rem)]">
            <ChromeTile block={block} grow={grow} />
          </div>
        </Container>

        <m.p style={{ opacity: hint }} className="label absolute bottom-5 left-1/2 z-10 -translate-x-1/2">
          Scroll ↓
        </m.p>
        <Bloom progress={bloom} origin={block} color="#e7eaee" />
      </div>
    </section>
  );
}
