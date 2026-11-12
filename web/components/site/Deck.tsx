"use client";

import { AnimatePresence, m, useInView, useReducedMotion } from "motion/react";
import Image from "next/image";
import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from "react";

import { deck } from "@/content/site";
import { slot, wrap } from "@/lib/fan";

import { cards } from "./cards";
import { ArrowRight, Crown, Sparkle } from "./doodles";
import { Spray } from "./Spray";

const AUTOPLAY_MS = 3800;
const spring = { type: "spring", stiffness: 120, damping: 18, mass: 0.9 } as const;

/**
 * The six cards, fanned. Whichever is in front is the one described underneath. Moves on by
 * itself while it is on screen and nobody is touching it; drag it, click a card, use the arrows
 * or the arrow keys.
 */
export function Deck() {
  const count = deck.cards.length;
  const [active, setActive] = useState(1);
  const [held, setHeld] = useState(false);
  const stage = useRef<HTMLDivElement>(null);
  const inView = useInView(stage, { amount: 0.5 });
  const reduce = useReducedMotion();

  const go = useCallback((by: number) => setActive((a) => wrap(a + by, count)), [count]);

  useEffect(() => {
    if (!inView || held || reduce) return;
    const t = window.setTimeout(() => go(1), AUTOPLAY_MS);
    return () => window.clearTimeout(t);
  }, [active, inView, held, reduce, go]);

  function onKey(e: KeyboardEvent) {
    if (e.key === "ArrowRight") {
      e.preventDefault();
      go(1);
    } else if (e.key === "ArrowLeft") {
      e.preventDefault();
      go(-1);
    }
  }

  const card = deck.cards[active];

  return (
    <section id="deck" aria-labelledby="deck-title" className="relative scroll-mt-16 overflow-hidden py-24 lg:py-32">
      <div className="mx-auto flex max-w-page flex-col gap-6 px-4 sm:px-6 lg:flex-row lg:items-end lg:justify-between lg:px-8">
        <h2 id="deck-title" className="shout text-[clamp(2.8rem,5.6vw,5.8rem)] lg:whitespace-nowrap">
          {deck.lead} <Spray delay={0.4}>{deck.sprayed}</Spray>
        </h2>
        <p className="max-w-[19rem] text-[0.95rem] leading-relaxed text-dust lg:pb-3">{deck.body}</p>
      </div>

      <div
        role="group"
        aria-roledescription="carousel"
        aria-label="Zeno's features, one card each"
        tabIndex={0}
        onKeyDown={onKey}
        onPointerEnter={() => setHeld(true)}
        onPointerLeave={() => setHeld(false)}
        onFocus={() => setHeld(true)}
        onBlur={() => setHeld(false)}
        className="relative mx-auto mt-14 max-w-page rounded-2xl px-4 outline-offset-8 sm:px-6 lg:mt-16 lg:px-8"
      >
        <Crown className="absolute left-[8%] top-0 w-14 -rotate-[12deg] text-violet-hi lg:left-[14%] lg:w-20" />
        <Crown className="absolute right-[9%] top-[-1.5rem] w-12 rotate-[8deg] text-cream/60 lg:right-[16%] lg:w-16" />
        <Sparkle className="absolute bottom-[30%] right-[6%] w-8 text-spray lg:right-[12%]" />

        <m.div
          ref={stage}
          drag="x"
          dragConstraints={{ left: 0, right: 0 }}
          dragElastic={0.18}
          dragSnapToOrigin
          onDragEnd={(_, info) => {
            if (info.offset.x < -50 || info.velocity.x < -400) go(1);
            else if (info.offset.x > 50 || info.velocity.x > 400) go(-1);
          }}
          className="relative mx-auto h-[25rem] cursor-grab touch-pan-y active:cursor-grabbing sm:h-[29rem] lg:h-[33rem]"
        >
          {deck.cards.map((c, i) => {
            const s = slot(i, active, count);
            const front = s.offset === 0;
            return (
              <m.button
                key={c.id}
                type="button"
                tabIndex={-1}
                aria-label={front ? `${c.name}, ${c.feature}` : `Show ${c.name}`}
                aria-current={front ? "true" : undefined}
                onClick={() => setActive(i)}
                initial={false}
                animate={{
                  x: `${s.x * 100}%`,
                  y: `${s.y * 100}%`,
                  rotate: s.rotate,
                  scale: s.scale,
                  opacity: s.visible ? 1 : 0,
                }}
                transition={spring}
                style={{ zIndex: s.z }}
                className={`absolute left-1/2 top-4 -ml-[min(29vw,8.4rem)] w-[min(58vw,16.8rem)] sm:-ml-[9.5rem] sm:w-[19rem] lg:-ml-[11rem] lg:w-[22rem] ${s.visible ? "" : "pointer-events-none"}`}
              >
                <Image
                  src={cards[c.image]}
                  alt={c.alt}
                  sizes="(min-width: 1024px) 352px, (min-width: 640px) 304px, 58vw"
                  draggable={false}
                  className={`h-auto w-full select-none transition-[filter] duration-500 ${front ? "drop-shadow-[0_30px_45px_rgba(0,0,0,0.7)]" : "brightness-[0.55] saturate-[0.8]"}`}
                />
              </m.button>
            );
          })}
        </m.div>

        <div className="relative z-20 mx-auto mt-2 flex max-w-[44rem] items-center gap-3">
          <button
            type="button"
            onClick={() => go(-1)}
            aria-label="Previous card"
            className="grid size-11 shrink-0 place-items-center rounded-md border border-cream/30 text-cream transition-colors hover:border-spray hover:text-spray"
          >
            <ArrowRight className="size-5 rotate-180" />
          </button>
          <div aria-live="polite" className="min-w-0 flex-1 rounded-md border border-cream/30 bg-coal/90 px-4 py-3 text-center">
            <AnimatePresence mode="wait" initial={false}>
              <m.div key={card.id} initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: -8 }} transition={{ duration: 0.22 }}>
                <p className="caps flex flex-wrap items-baseline justify-center gap-x-4 gap-y-1 text-[0.8rem]">
                  <span>
                    Card <span className="text-violet-hi">0{active + 1}</span>
                  </span>
                  <span className="text-cream">{card.name}</span>
                  <span className="text-violet-hi">{card.feature}</span>
                  <span>
                    {card.stat.k} <span className="text-spray">{card.stat.v}</span>
                  </span>
                </p>
                <p className="mt-1.5 text-[0.88rem] leading-snug text-dust">{card.line}</p>
              </m.div>
            </AnimatePresence>
          </div>
          <button
            type="button"
            onClick={() => go(1)}
            aria-label="Next card"
            className="grid size-11 shrink-0 place-items-center rounded-md border border-cream/30 text-cream transition-colors hover:border-spray hover:text-spray"
          >
            <ArrowRight className="size-5" />
          </button>
        </div>
      </div>
    </section>
  );
}
