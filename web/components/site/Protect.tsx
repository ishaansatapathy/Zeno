"use client";

import { m, useScroll, useSpring } from "motion/react";
import Image from "next/image";
import { useRef } from "react";

import { protect } from "@/content/site";

import { cards } from "./cards";
import { Sparkle } from "./doodles";
import { Spray } from "./Spray";

/**
 * The cream panel, with a line of violet spray paint looping around it as you scroll past, the
 * way a crew rings the piece they are proud of.
 */
export function Protect() {
  const ref = useRef<HTMLDivElement>(null);
  const { scrollYProgress } = useScroll({ target: ref, offset: ["start 0.95", "end 0.35"] });
  const drawn = useSpring(scrollYProgress, { stiffness: 80, damping: 22, restDelta: 0.001 });

  return (
    <section aria-labelledby="protect-title" className="relative mx-auto max-w-page px-4 py-20 sm:px-6 lg:px-8 lg:py-28">
      <h2 id="protect-title" className="shout text-[clamp(2.8rem,6.6vw,6.6rem)] lg:whitespace-nowrap">
        Every second, <Spray delay={0.4}>locked</Spray>
      </h2>

      <div ref={ref} className="relative mt-16 lg:mt-24">
        {/* The spray: a fat violet line roughened at the edges, and a thin yellow one riding it. */}
        <svg
          viewBox="0 0 1000 560"
          preserveAspectRatio="none"
          aria-hidden="true"
          focusable="false"
          className="pointer-events-none absolute -inset-x-3 -inset-y-10 z-0 h-[calc(100%+5rem)] w-[calc(100%+1.5rem)] overflow-visible lg:-inset-x-10 lg:z-20 lg:w-[calc(100%+5rem)]"
        >
          <defs>
            <filter id="spray-edge" x="-10%" y="-10%" width="120%" height="120%">
              <feTurbulence type="fractalNoise" baseFrequency="0.9" numOctaves="2" seed="7" />
              <feDisplacementMap in="SourceGraphic" scale="7" />
            </filter>
          </defs>
          <m.path
            d="M120 18C46 40 8 160 18 300c10 140 70 236 260 246 210 10 460-4 620-30 80-14 98-96 86-220C972 160 946 52 860 26 780 4 640 30 560 12"
            fill="none"
            stroke="#8b5cf6"
            strokeWidth={13}
            strokeLinecap="round"
            vectorEffect="non-scaling-stroke"
            filter="url(#spray-edge)"
            style={{ pathLength: drawn }}
          />
          <m.path
            d="M134 34C70 58 34 168 42 300c8 126 60 216 240 226 200 10 440-6 600-30 66-10 84-84 74-196"
            fill="none"
            stroke="#ffc21a"
            strokeWidth={3}
            strokeLinecap="round"
            vectorEffect="non-scaling-stroke"
            style={{ pathLength: drawn }}
          />
        </svg>

        <div className="halftone absolute -left-10 -top-12 size-48 rounded-full text-violet-hi/25 [mask-image:radial-gradient(closest-side,black,transparent)]" />

        <div className="relative z-10 grid rounded-[6px] bg-cream text-ink lg:grid-cols-[minmax(0,1fr)_minmax(0,0.9fr)]">
          <div className="flex flex-col p-7 sm:p-10 lg:p-14">
            <ul className="space-y-6">
              {protect.points.map((p) => (
                <li key={p} className="flex max-w-[31rem] gap-3 text-[1rem] leading-relaxed lg:text-[1.1rem]">
                  <span aria-hidden="true" className="mt-[0.15em] text-violet-deep">
                    ▸
                  </span>
                  {p}
                </li>
              ))}
            </ul>
            <p className="caps mt-12 text-[0.95rem] lg:mt-auto lg:pt-14">
              {protect.foot.k} <span aria-hidden="true">·</span> <span className="text-violet-deep">{protect.foot.v}</span>
            </p>
          </div>
          <div className="relative flex items-center justify-center px-8 pb-10 lg:p-0">
            <m.div
              initial={{ opacity: 0, rotate: 16, y: 60 }}
              whileInView={{ opacity: 1, rotate: 5, y: 0 }}
              viewport={{ once: true, margin: "0px 0px -15% 0px" }}
              transition={{ type: "spring", stiffness: 70, damping: 14 }}
              className="relative w-[min(68vw,19rem)] lg:-my-14 lg:w-[22rem]"
            >
              <Image src={cards[3]} alt={protect.alt} sizes="(min-width: 1024px) 352px, 68vw" className="h-auto w-full drop-shadow-[0_28px_36px_rgba(12,10,15,0.55)]" />
              <Sparkle className="absolute -right-6 -top-6 w-10 text-violet" />
            </m.div>
          </div>
        </div>
      </div>
    </section>
  );
}
