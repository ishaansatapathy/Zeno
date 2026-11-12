"use client";

import { AnimatePresence, m, useInView, useReducedMotion } from "motion/react";
import Image from "next/image";
import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from "react";

import { deck } from "@/content/site";
import { slot, wrap } from "@/lib/fan";

import { cards } from "./cards";
import { Arrow, ArrowRight, Crown, Sparkle } from "./doodles";
import { Ring, Splatter, Tape } from "./graffiti";
import { Spray } from "./Spray";

const AUTOPLAY_MS = 3800;
const spring = { type: "spring", stiffness: 120, damping: 18, mass: 0.9 } as const;

/**
 * One card's width at each size. On wide screens it follows the screen's height too, so the fan,
 * the card's name and its description fit on one screen even on a short laptop window.
 */
const cardWidth = "w-[min(58vw,16.8rem)] sm:w-[19rem] lg:w-[min(21rem,38svh)]";
const cardCentre = "-ml-[min(29vw,8.4rem)] sm:-ml-[9.5rem] lg:-ml-[min(10.5rem,19svh)]";
const stageHeight = "h-[calc(min(58vw,16.8rem)*1.34+2rem)] sm:h-[calc(19rem*1.34+2rem)] lg:h-[calc(min(21rem,38svh)*1.34+2rem)]";

function MarkerButton({ dir, onClick }: { dir: "prev" | "next"; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={dir === "prev" ? "Previous card" : "Next card"}
      className="group relative grid size-14 shrink-0 place-items-center rounded-full text-cream transition-colors hover:text-spray"
    >
      <Ring className={`absolute inset-0 size-full transition-transform duration-500 ease-out group-hover:rotate-[150deg] ${dir === "prev" ? "-scale-x-100" : ""}`} />
      <ArrowRight className={`relative size-6 ${dir === "prev" ? "rotate-180" : ""}`} />
    </button>
  );
}

/**
 * The six cards, fanned on a wall. Whichever is in front gets its name sprayed across it and is
 * described on the slip underneath. Moves on by itself while it is on screen and nobody is
 * touching it; drag it, click a card, use the arrows or the arrow keys.
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
    <section id="deck" aria-labelledby="deck-title" className="relative scroll-mt-16 overflow-hidden py-24 lg:py-28">
      <div aria-hidden="true" className="bricks pointer-events-none absolute inset-0 -z-10 [mask-image:radial-gradient(ellipse_60%_55%_at_50%_55%,black_20%,transparent_75%)]" />
      <div aria-hidden="true" className="pointer-events-none absolute left-1/2 top-[55%] -z-10 size-[60vmin] -translate-x-1/2 -translate-y-1/2 rounded-full bg-violet/15 blur-[110px]" />

      <div className="mx-auto flex max-w-page flex-col gap-6 px-4 sm:px-6 lg:flex-row lg:items-end lg:justify-between lg:px-8">
        <h2 id="deck-title" className="shout text-[clamp(2.8rem,5.6vw,5.8rem)] lg:whitespace-nowrap">
          {deck.lead}{" "}
          <Spray delay={0.4} drips>
            {deck.sprayed}
          </Spray>
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
        className="relative mx-auto mt-14 max-w-page rounded-2xl px-4 outline-offset-8 sm:px-6 lg:mt-12 lg:px-8"
      >
        <Crown className="absolute left-[8%] top-0 w-14 -rotate-[12deg] text-violet-hi lg:left-[13%] lg:w-20" />
        <Splatter className="absolute left-[4%] top-[34%] hidden w-24 text-spray/60 lg:block" />
        <Splatter className="absolute bottom-[26%] right-[5%] hidden w-20 rotate-45 text-violet/70 lg:block" />
        <Sparkle className="absolute right-[9%] top-[42%] w-8 text-spray lg:right-[15%]" />
        <p aria-hidden="true" className="absolute right-[7%] top-[-0.5rem] hidden rotate-[7deg] items-end gap-1 text-cream/80 lg:flex">
          <span className="tag text-[2rem]">drag me</span>
          <Arrow className="mb-[-2.6rem] w-20 rotate-[150deg] -scale-y-100 text-cream/70" />
        </p>

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
          className={`relative mx-auto cursor-grab touch-pan-y active:cursor-grabbing ${stageHeight}`}
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
                className={`absolute left-1/2 top-3 ${cardCentre} ${cardWidth} ${s.visible ? "" : "pointer-events-none"}`}
              >
                <Image
                  src={cards[c.image]}
                  alt={c.alt}
                  sizes="(min-width: 1024px) 336px, (min-width: 640px) 304px, 58vw"
                  draggable={false}
                  className="h-auto w-full select-none drop-shadow-[0_26px_32px_rgba(0,0,0,0.6)]"
                />
                <span
                  aria-hidden="true"
                  className={`absolute inset-0 rounded-[5.5%/4.125%] bg-ink transition-opacity duration-500 ${front ? "opacity-0" : "opacity-40"}`}
                />
              </m.button>
            );
          })}
        </m.div>

        {/* The name, sprayed across the bottom of the card in front, and the slip under it. */}
        <div className="relative z-20 mx-auto -mt-14 max-w-[42rem] sm:-mt-16">
          <div aria-hidden="true" className="grid h-[clamp(3.6rem,6.5vw,5.4rem)] place-items-center">
            <AnimatePresence mode="wait" initial={false}>
              <m.p
                key={card.id}
                className="tag -rotate-[4deg] whitespace-nowrap text-[clamp(3.2rem,6vw,5rem)] text-spray [text-shadow:3px_3px_0_#0c0a0f,-1px_-1px_0_#0c0a0f]"
                initial={{ clipPath: "inset(-40% 100% -40% -6%)" }}
                animate={{ clipPath: "inset(-40% -6% -40% -6%)" }}
                exit={{ opacity: 0, transition: { duration: 0.15 } }}
                transition={{ duration: 0.5, ease: [0.65, 0, 0.35, 1] }}
              >
                {card.name}
              </m.p>
            </AnimatePresence>
          </div>

          <div className="mt-5 flex flex-wrap items-center justify-center gap-x-6 gap-y-5 sm:flex-nowrap sm:gap-5">
            <MarkerButton dir="prev" onClick={() => go(-1)} />
            <div aria-live="polite" className="relative order-first w-full min-w-0 -rotate-1 sm:order-none sm:w-auto sm:flex-1 rounded-[3px] bg-cream px-5 pb-4 pt-5 text-center text-ink shadow-[5px_5px_0_#8b5cf6]">
              <Tape className="-top-3 left-6 -rotate-[8deg]" />
              <Tape className="-top-3 right-6 rotate-[6deg]" />
              <AnimatePresence mode="wait" initial={false}>
                <m.div key={card.id} initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: -6 }} transition={{ duration: 0.2 }}>
                  <p className="caps flex flex-wrap items-baseline justify-center gap-x-4 gap-y-1 text-[0.8rem]">
                    <span>
                      Card <span className="text-violet-deep">0{active + 1}</span>
                    </span>
                    <span className="sr-only">{card.name}</span>
                    <span className="text-violet-deep">{card.feature}</span>
                    <span>
                      {card.stat.k} <span className="text-violet-deep">{card.stat.v}</span>
                    </span>
                  </p>
                  <p className="mt-1.5 text-[0.92rem] leading-snug text-ink/80">{card.line}</p>
                </m.div>
              </AnimatePresence>
            </div>
            <MarkerButton dir="next" onClick={() => go(1)} />
          </div>
        </div>
      </div>
    </section>
  );
}
