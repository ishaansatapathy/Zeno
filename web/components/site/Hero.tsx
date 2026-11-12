"use client";

import { m, useScroll, useTransform } from "motion/react";
import { useRef } from "react";

import { hero } from "@/content/site";

import { Bloom } from "./Bloom";
import { Stream } from "./Stream";

/**
 * The opening: a statement, and under it raw video flowing into the orange block and leaving as
 * locked segments. The hero is pinned for a while as you scroll: the words fade, the streams
 * tighten, the block grows, and then orange floods out from it square by square until the screen
 * is the orange section that follows.
 */
export function Hero() {
  const ref = useRef<HTMLElement>(null);
  const block = useRef<HTMLDivElement>(null);
  const { scrollYProgress } = useScroll({ target: ref, offset: ["start start", "end end"] });

  const wordsOpacity = useTransform(scrollYProgress, [0.04, 0.3], [1, 0]);
  const wordsY = useTransform(scrollYProgress, [0, 0.3], [0, -70]);
  const hint = useTransform(scrollYProgress, [0, 0.06], [1, 0]);
  const pull = useTransform(scrollYProgress, [0, 0.45], [0, 1]);
  const grow = useTransform(scrollYProgress, [0.08, 0.42], [1, 1.9]);
  const bloom = useTransform(scrollYProgress, [0.38, 0.9], [0, 1]);

  return (
    <section ref={ref} aria-labelledby="hero-title" className="relative h-[240svh]">
      <div className="sticky top-0 h-[100svh] overflow-hidden">
        <m.div style={{ opacity: wordsOpacity, y: wordsY }} className="relative z-10 mx-auto max-w-5xl px-5 pt-[clamp(6.5rem,17svh,10rem)] text-center">
          <h1 id="hero-title" className="display text-[clamp(2.6rem,6.2vw,5.6rem)]">
            <span className="rise block">{hero.title[0]}</span>
            <em className="rise block italic text-signal" style={{ ["--delay" as string]: "0.12s" }}>
              {hero.title[1]}
            </em>
          </h1>
          <p className="rise mx-auto mt-7 max-w-[34rem] text-[0.9rem] leading-relaxed text-paper/75" style={{ ["--delay" as string]: "0.3s" }}>
            {hero.lines[0]} {hero.lines[1]} <span className="text-signal">{hero.accentLine}</span>
          </p>
        </m.div>

        <div className="absolute inset-x-0 bottom-[3svh] top-[clamp(17rem,42svh,27rem)]">
          <Stream pull={pull} block={block} />
          <div className="absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2">
            <m.div
              ref={block}
              style={{ scale: grow }}
              className="grid size-14 place-items-center bg-signal shadow-[0_0_60px_rgba(255,91,20,0.35)] lg:size-[4.5rem]"
            >
              <svg viewBox="0 0 20 20" aria-hidden="true" focusable="false" className="size-[42%]">
                <path d="M6 3.5v13l11-6.5L6 3.5Z" fill="#050505" />
              </svg>
            </m.div>
          </div>
        </div>

        <m.p style={{ opacity: hint }} className="label absolute bottom-5 left-1/2 z-10 -translate-x-1/2">
          {hero.scroll} ↓
        </m.p>
        <Bloom progress={bloom} origin={block} />
      </div>
    </section>
  );
}
