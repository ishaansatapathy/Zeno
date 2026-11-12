"use client";

import { m, useMotionValue, useScroll, useSpring, useTransform } from "motion/react";
import Image from "next/image";
import { useRef, type PointerEvent, type ReactNode } from "react";

import { hero, links } from "@/content/site";

import { cards } from "./cards";
import { Arrow, Crown, Sparkle } from "./doodles";
import { DripButton } from "./DripButton";
import { Spray } from "./Spray";

function Rise({ children, delay, className = "" }: { children: ReactNode; delay: number; className?: string }) {
  return (
    <span className={`rise block ${className}`} style={{ ["--delay" as string]: `${delay}s` }}>
      {children}
    </span>
  );
}

/** The card's own corner radius (5.5% of its width), for anything laid over it. */
const cardCorners = "rounded-[5.5%/4.125%]";

export function Hero() {
  const ref = useRef<HTMLElement>(null);

  // The pointer, as -0.5..0.5 across the hero. The card leans towards it.
  const px = useMotionValue(0);
  const py = useMotionValue(0);
  const spring = { stiffness: 110, damping: 16, mass: 0.6 };
  const rotateX = useSpring(useTransform(py, [-0.5, 0.5], [10, -10]), spring);
  const rotateY = useSpring(useTransform(px, [-0.5, 0.5], [-14, 14]), spring);
  const sheen = useSpring(useTransform(px, [-0.5, 0.5], [90, 10]), spring);
  const sheenPosition = useTransform(sheen, (v) => `${v}% 0%`);

  // Scrolling away, the card drifts up and turns, as if tossed onto the next section.
  const { scrollYProgress } = useScroll({ target: ref, offset: ["start start", "end start"] });
  const lift = useTransform(scrollYProgress, [0, 1], [0, -160]);
  const turn = useTransform(scrollYProgress, [0, 1], [-4, -16]);
  const sideL = useTransform(scrollYProgress, [0, 1], [0, -220]);
  const sideR = useTransform(scrollYProgress, [0, 1], [0, -320]);

  function onMove(e: PointerEvent<HTMLElement>) {
    if (e.pointerType !== "mouse" || !ref.current) return;
    const r = ref.current.getBoundingClientRect();
    px.set((e.clientX - r.left) / r.width - 0.5);
    py.set((e.clientY - r.top) / r.height - 0.5);
  }
  function onLeave() {
    px.set(0);
    py.set(0);
  }

  return (
    <section
      ref={ref}
      onPointerMove={onMove}
      onPointerLeave={onLeave}
      aria-labelledby="hero-title"
      className="relative isolate flex min-h-[100svh] flex-col overflow-hidden pt-16 lg:max-h-[68rem]"
    >
      {/* The room the card hangs in: a violet glow, a stencil of dots, two neon tubes. */}
      <div aria-hidden="true" className="pointer-events-none absolute inset-0 -z-10">
        <div className="absolute left-1/2 top-[36%] size-[70vmin] -translate-x-1/2 -translate-y-1/2 rounded-full bg-violet/20 blur-[100px]" />
        <div className="halftone absolute left-[56%] top-[9%] size-[26vmin] rounded-full text-spray/25 [mask-image:radial-gradient(closest-side,black,transparent)]" />
        <div className="halftone absolute left-[30%] top-[40%] size-[20vmin] rounded-full text-violet-hi/25 [mask-image:radial-gradient(closest-side,black,transparent)]" />
        <div className="absolute left-[33%] top-[7%] hidden h-[46%] w-[3px] rounded-full bg-violet-hi/60 shadow-[0_0_22px_6px_rgba(139,92,246,0.45)] lg:block" />
        <div className="absolute right-[33%] top-[7%] hidden h-[46%] w-[3px] rounded-full bg-violet-hi/60 shadow-[0_0_22px_6px_rgba(139,92,246,0.45)] lg:block" />
      </div>

      {/* Two more cards, half out of frame, thrown on the table. */}
      <m.div aria-hidden="true" style={{ y: sideL }} className="absolute -left-16 top-[17%] hidden w-[clamp(150px,13vw,210px)] -rotate-[16deg] opacity-80 lg:block">
        <Image src={cards[4]} alt="" sizes="210px" className="h-auto w-full" />
      </m.div>
      <m.div aria-hidden="true" style={{ y: sideR }} className="absolute -right-14 top-[30%] hidden w-[clamp(150px,13vw,210px)] rotate-[18deg] opacity-80 lg:block">
        <Image src={cards[5]} alt="" sizes="210px" className="h-auto w-full" />
      </m.div>

      {/* The hero card. */}
      <div className="relative mx-auto mt-8 w-[min(62vw,280px)] lg:absolute lg:left-1/2 lg:top-[10.5%] lg:mt-0 lg:w-[clamp(230px,19.5vw,310px)] lg:-translate-x-1/2">
        <m.div style={{ y: lift, rotate: turn }}>
          <m.div
            initial={{ opacity: 0, y: -80, rotate: 14, scale: 0.86 }}
            animate={{ opacity: 1, y: 0, rotate: 0, scale: 1 }}
            transition={{ type: "spring", stiffness: 80, damping: 13, delay: 0.1 }}
          >
            <m.div style={{ rotateX, rotateY, transformPerspective: 900 }}>
              <m.div animate={{ y: [0, -10, 0] }} transition={{ duration: 6, repeat: Infinity, ease: "easeInOut" }} className="relative">
                <Image
                  src={cards[2]}
                  alt="Zeno's mascot, a robot with a monitor for a face, sitting cross-legged on a video player and throwing a peace sign."
                  priority
                  sizes="(min-width: 1024px) 310px, 62vw"
                  className="h-auto w-full drop-shadow-[0_34px_40px_rgba(0,0,0,0.65)]"
                />
                <m.div
                  aria-hidden="true"
                  style={{ backgroundPosition: sheenPosition }}
                  className={`absolute inset-0 ${cardCorners} bg-[linear-gradient(115deg,transparent_35%,rgba(255,255,255,0.28)_50%,transparent_65%)] bg-[length:260%_100%] mix-blend-overlay`}
                />
                <div className="absolute -bottom-6 left-1/2 w-[94%] -translate-x-1/2 -rotate-[2deg] rounded-md border border-cream/30 bg-ink/95 px-3 pb-1.5 pt-2 shadow-[0_10px_30px_rgba(0,0,0,0.5)]">
                  <dl className="flex justify-between gap-2">
                    {hero.plate.map((s) => (
                      <div key={s.k} className="flex items-baseline gap-1.5">
                        <dt className="caps text-[0.7rem] text-dust">{s.k}</dt>
                        <dd className="shout text-lg text-spray">{s.v}</dd>
                      </div>
                    ))}
                  </dl>
                  <p className="caps mt-1 border-t border-cream/15 pt-1 text-center text-[0.62rem] tracking-[0.18em] text-violet-hi">
                    — {hero.plateFoot} —
                  </p>
                </div>
              </m.div>
            </m.div>
          </m.div>
        </m.div>
        <Crown className="absolute -left-14 -top-3 w-12 -rotate-[14deg] text-spray lg:-left-20 lg:w-16" />
        <Sparkle className="absolute -right-10 top-[18%] w-8 text-cream lg:-right-16 lg:w-11" />
        <Sparkle className="absolute -right-4 top-[6%] w-4 text-violet-hi lg:-right-7 lg:w-5" />
        <Arrow className="absolute -left-24 bottom-[22%] hidden w-20 rotate-[8deg] text-violet-hi lg:block" />
      </div>

      {/* The words. */}
      <div className="relative mx-auto mt-auto w-full max-w-page px-4 pb-12 pt-16 sm:px-6 lg:px-8 lg:pb-16">
        <div className="flex flex-col gap-9 lg:flex-row lg:items-end lg:justify-between lg:gap-10">
          <div>
            <Rise delay={0.05} className="mb-4 text-[1.35rem] sm:text-2xl">
              <span className="flex items-center gap-2.5 text-violet-hi">
                <Crown className="w-7" />
                <span className="tag text-[1.35rem] sm:text-2xl">{hero.tag}</span>
              </span>
            </Rise>
            <h1 id="hero-title" className="shout text-[clamp(3.2rem,7.3vw,7.4rem)] lg:whitespace-nowrap">
              <Rise delay={0.1}>{hero.line1}</Rise>
              <Rise delay={0.2}>
                {hero.line2} <Spray delay={0.75}>{hero.sprayed}</Spray>
              </Rise>
            </h1>
          </div>
          <Rise delay={0.3} className="max-w-[20rem] text-[0.95rem] lg:pb-2">
            <span className="block text-[0.95rem] leading-relaxed text-dust">{hero.body}</span>
            <DripButton href={links.start} className="mt-7">
              {hero.cta}
            </DripButton>
          </Rise>
        </div>
      </div>
    </section>
  );
}
