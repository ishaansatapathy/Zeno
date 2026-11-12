"use client";

import { m, useScroll, useSpring, useTransform, type MotionValue } from "motion/react";
import { useLayoutEffect, useRef, useState, type ReactNode } from "react";

import { useMedia } from "../useMedia";

export type TrackCard = { key: string; label: string; title: string; body: ReactNode };

function Face({ card, index, total }: { card: TrackCard; index: number; total: number }) {
  return (
    <article className="metal flex h-full w-full flex-col p-7">
      <div className="flex items-center justify-between font-mono text-[0.72rem] text-text-muted">
        <span>{card.label}</span>
        <span>
          {String(index + 1).padStart(2, "0")} / {String(total).padStart(2, "0")}
        </span>
      </div>
      <h3 className="headline silver mt-4 text-[1.9rem]">{card.title}</h3>
      <div className="mt-6 min-h-0 flex-1">{card.body}</div>
    </article>
  );
}

/**
 * One card on the track. As it crosses the middle of the screen it faces you squarely; on either
 * side it turns a little away and settles back, like cards on a carousel seen from in front.
 */
function Turning({ progress, center, children }: { progress: MotionValue<number>; center: number; children: ReactNode }) {
  const span = 0.42;
  const rotateY = useTransform(progress, [center - span, center, center + span], [16, 0, -16]);
  const scale = useTransform(progress, [center - span, center, center + span], [0.93, 1, 0.93]);
  const opacity = useTransform(progress, [center - span, center, center + span], [0.55, 1, 0.55]);
  return (
    <m.div style={{ rotateY, scale, opacity }} className="h-[30rem] w-[min(34rem,80vw)] shrink-0 [transform-style:preserve-3d]">
      {children}
    </m.div>
  );
}

/** Desktop: the page scrolls down, the cards travel left, in perspective. */
function Pinned({ cards, header }: { cards: TrackCard[]; header: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  const track = useRef<HTMLDivElement>(null);
  const [distance, setDistance] = useState(0);
  const { scrollYProgress } = useScroll({ target: ref, offset: ["start start", "end end"] });
  const smooth = useSpring(scrollYProgress, { stiffness: 120, damping: 30, mass: 0.5 });
  const x = useTransform(smooth, [0, 1], [0, -distance]);

  useLayoutEffect(() => {
    const measure = () => {
      if (track.current) setDistance(Math.max(0, track.current.scrollWidth - window.innerWidth + 64));
    };
    measure();
    window.addEventListener("resize", measure);
    return () => window.removeEventListener("resize", measure);
  }, []);

  const n = cards.length;
  return (
    <div ref={ref} className="relative" style={{ height: `${n * 75}vh` }}>
      <div className="sticky top-[4.5rem] flex h-[calc(100dvh-4.5rem)] flex-col justify-center gap-10 overflow-hidden">
        <div className="mx-auto w-full max-w-page px-8">{header}</div>
        <div className="[perspective:1600px]">
          <m.div ref={track} style={{ x }} className="flex w-max gap-6 pl-[max(2rem,calc((100vw-76rem)/2+2rem))] pr-8">
            {cards.map((card, i) => (
              <Turning key={card.key} progress={smooth} center={n === 1 ? 0.5 : i / (n - 1)}>
                <Face card={card} index={i} total={n} />
              </Turning>
            ))}
          </m.div>
        </div>
      </div>
    </div>
  );
}

/** Phones and reduced motion: a row you swipe, snapping card by card. */
function Swipe({ cards, header }: { cards: TrackCard[]; header: ReactNode }) {
  return (
    <div className="flex flex-col gap-8 py-24">
      <div className="mx-auto w-full max-w-page px-5 sm:px-8">{header}</div>
      <div className="flex snap-x snap-mandatory gap-4 overflow-x-auto px-5 pb-4 [scrollbar-width:none] sm:px-8">
        {cards.map((card, i) => (
          <div key={card.key} className="h-[28rem] w-[85vw] max-w-[26rem] shrink-0 snap-center">
            <Face card={card} index={i} total={cards.length} />
          </div>
        ))}
      </div>
    </div>
  );
}

export function CardTrack({ cards, header }: { cards: TrackCard[]; header: ReactNode }) {
  const wide = useMedia("(min-width: 1024px) and (prefers-reduced-motion: no-preference)");
  return wide ? <Pinned cards={cards} header={header} /> : <Swipe cards={cards} header={header} />;
}
