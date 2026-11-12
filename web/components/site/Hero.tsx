"use client";

import { m, useMotionValue, useScroll, useSpring, useTransform } from "motion/react";
import Image from "next/image";
import { useRef, type PointerEvent, type ReactNode } from "react";

import { hero, links } from "@/content/site";

import { cards } from "./cards";
import { Arrow, Crown, Sparkle } from "./doodles";
import { DripButton } from "./DripButton";
import { HelloSticker, Splatter, Tape } from "./graffiti";
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

/**
 * A card taped to a brick wall, a name sticker slapped on it, the headline sprayed underneath.
 * On wide screens the card is sized by the screen's height, not its width, so the card and the
 * headline share one screen without ever overlapping, however short the window is.
 */
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
  const lift = useTransform(scrollYProgress, [0, 1], [0, -140]);
  const turn = useTransform(scrollYProgress, [0, 1], [-4, -14]);
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
      className="relative isolate flex min-h-[100svh] flex-col overflow-hidden pt-16 lg:min-h-[max(100svh,40rem)]"
    >
      {/* The wall: bricks, a violet glow, an old tag in outline, splatter, stencil dots. */}
      <div aria-hidden="true" className="pointer-events-none absolute inset-0 -z-10">
        <div className="bricks absolute inset-0 [mask-image:radial-gradient(ellipse_75%_70%_at_50%_38%,black_25%,transparent_75%)]" />
        <div className="absolute left-1/2 top-[34%] size-[70vmin] -translate-x-1/2 -translate-y-1/2 rounded-full bg-violet/20 blur-[100px]" />
        <span className="wall-tag absolute right-[5%] top-[9%] hidden -rotate-[9deg] select-none text-[clamp(9rem,20vw,19rem)] lg:block">
          zeno
        </span>
        <Splatter className="absolute left-[22%] top-[12%] hidden w-28 text-violet/60 lg:block" />
        <Splatter className="absolute right-[24%] top-[52%] hidden w-16 rotate-90 text-spray/50 lg:block" />
        <div className="halftone absolute left-[57%] top-[8%] size-[24vmin] rounded-full text-spray/25 [mask-image:radial-gradient(closest-side,black,transparent)]" />
        <div className="halftone absolute left-[31%] top-[36%] size-[18vmin] rounded-full text-violet-hi/25 [mask-image:radial-gradient(closest-side,black,transparent)]" />
      </div>

      {/* Two more cards, half out of frame, thrown on the table. */}
      <m.div aria-hidden="true" style={{ y: sideL }} className="absolute -left-16 top-[17%] hidden w-[clamp(140px,12vw,200px)] -rotate-[16deg] opacity-80 lg:block">
        <Image src={cards[4]} alt="" sizes="200px" className="h-auto w-full" />
      </m.div>
      <m.div aria-hidden="true" style={{ y: sideR }} className="absolute -right-14 top-[36%] hidden w-[clamp(140px,12vw,200px)] rotate-[18deg] opacity-80 lg:block">
        <Image src={cards[5]} alt="" sizes="200px" className="h-auto w-full" />
      </m.div>

      {/* The hero card, taped up, with its name stuck on. */}
      <div className="relative mx-auto mt-8 w-[min(60vw,270px)] lg:mt-[3svh] lg:aspect-[3/4] lg:h-[clamp(16rem,calc(78svh-14rem),27rem)] lg:w-auto">
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
                  sizes="(min-width: 1024px) 330px, 60vw"
                  className="h-auto w-full drop-shadow-[0_34px_40px_rgba(0,0,0,0.65)]"
                />
                <m.div
                  aria-hidden="true"
                  style={{ backgroundPosition: sheenPosition }}
                  className={`absolute inset-0 ${cardCorners} bg-[linear-gradient(115deg,transparent_35%,rgba(255,255,255,0.28)_50%,transparent_65%)] bg-[length:260%_100%] mix-blend-overlay`}
                />
                <Tape className="-left-7 top-3 -rotate-[38deg]" />
                <Tape className="-right-7 top-3 rotate-[36deg]" />
                <HelloSticker
                  name="zeno"
                  className="absolute -right-10 bottom-[7%] origin-bottom-right rotate-[9deg] scale-[0.7] sm:scale-[0.8] lg:-right-24 lg:bottom-[9%] lg:scale-100"
                />
              </m.div>
            </m.div>
          </m.div>
        </m.div>
        <Crown className="absolute -left-14 -top-4 w-12 -rotate-[14deg] text-spray lg:-left-20 lg:w-16" />
        <Sparkle className="absolute -right-10 top-[16%] w-8 text-cream lg:-right-16 lg:w-11" />
        <Sparkle className="absolute -right-4 top-[5%] w-4 text-violet-hi lg:-right-7 lg:w-5" />
        <Arrow className="absolute -left-24 bottom-[30%] hidden w-20 rotate-[8deg] text-violet-hi lg:block" />
      </div>

      {/* The words. */}
      <div className="relative mx-auto mt-auto w-full max-w-page px-4 pb-10 pt-14 sm:px-6 lg:px-8 lg:pb-[5svh] lg:pt-[4svh]">
        <div className="flex flex-col gap-9 lg:flex-row lg:items-end lg:justify-between lg:gap-10">
          <div>
            <Rise delay={0.05} className="mb-4 text-[1.35rem] sm:text-2xl">
              <span className="flex items-center gap-2.5 text-violet-hi">
                <Crown className="w-7" />
                <span className="tag text-[1.35rem] sm:text-2xl">{hero.tag}</span>
              </span>
            </Rise>
            <h1 id="hero-title" className="shout text-[clamp(3.2rem,13vw,4.4rem)] sm:text-[clamp(3.2rem,7.3vw,7.4rem)] lg:whitespace-nowrap lg:text-[min(7.3vw,12svh,7.4rem)]">
              <Rise delay={0.1}>{hero.line1}</Rise>
              <Rise delay={0.2}>
                {hero.line2}{" "}
                <span className="relative inline-block">
                  <Spray delay={0.75} drips>
                    {hero.sprayed}
                  </Spray>
                  <Splatter className="absolute -right-[0.75em] top-[0.2em] w-[0.6em] text-spray/90" />
                </span>
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
