"use client";

import { m } from "motion/react";

import { process } from "@/content/site";

import { Crown, Drips, Sparkle } from "./doodles";
import { CrossOut, Splatter, Tape } from "./graffiti";
import { Spray } from "./Spray";

const ease = [0.22, 1, 0.36, 1] as const;

/** Each card lands at its own slight angle, like four posters pasted up in a hurry. */
const tilt = [-2.2, 1.6, -1.2, 2.4];
const drop = ["lg:mt-0", "lg:mt-7", "lg:mt-2", "lg:mt-9"];
const tapeAngle = [-5, 4, -3, 6];
/** Where each card's spray of colour sits, so no two cards look stamped from one mould. */
const blots = [
  "-right-12 -top-14 bg-violet/30",
  "-left-14 bottom-6 bg-spray/15",
  "-right-10 bottom-10 bg-violet/25",
  "-left-10 -top-12 bg-spray/15",
];

export function Process() {
  return (
    <section id="how" aria-labelledby="how-title" className="relative scroll-mt-16 overflow-hidden">
      <div aria-hidden="true" className="bricks pointer-events-none absolute inset-0 -z-10 [mask-image:radial-gradient(ellipse_70%_60%_at_30%_25%,black_15%,transparent_70%)]" />
      <div className="mx-auto max-w-page px-4 py-24 sm:px-6 lg:px-8 lg:py-32">
        <h2 id="how-title" className="shout text-[clamp(3rem,8.4vw,8.6rem)]">
          <span className="block">
            {process.lead}{" "}
            <span className="relative inline-block text-cream/25">
              {process.struck}
              <CrossOut delay={0.2} className="absolute -left-[3%] top-[8%] h-[80%] w-[106%]" />
            </span>
          </span>
          <span className="mt-[0.18em] block">
            {process.mid}{" "}
            <span className="relative inline-block">
              <Spray delay={0.9} drips size="1.12em">
                {process.sprayed}
              </Spray>
              <Splatter className="absolute -right-[0.7em] top-[-0.1em] w-[0.7em] text-violet/80" />
            </span>
          </span>
        </h2>

        <ol className="mt-20 grid gap-10 sm:grid-cols-2 sm:gap-x-6 sm:gap-y-14 lg:mt-24 lg:grid-cols-4 lg:gap-5">
          {process.steps.map((s, i) => (
            <m.li
              key={s.n}
              className={drop[i]}
              initial={{ opacity: 0, y: 70, rotate: 0 }}
              whileInView={{ opacity: 1, y: 0, rotate: tilt[i] }}
              viewport={{ once: true, margin: "0px 0px -10% 0px" }}
              transition={{ duration: 0.9, delay: i * 0.1, ease }}
            >
              <div
                className="slab group relative flex h-full min-h-[23rem] flex-col p-6 pb-6 transition-transform duration-500 ease-out hover:-translate-y-2 hover:rotate-[calc(var(--r)*-1)]"
                style={{ ["--r" as string]: `${tilt[i]}deg` }}
              >
                {/* Paint on the poster: a blot of colour and a patch of stencil dots. */}
                <div aria-hidden="true" className="pointer-events-none absolute inset-0 overflow-hidden rounded-[14px]">
                  <div className={`absolute size-44 rounded-full blur-2xl ${blots[i]}`} />
                  <div className="halftone absolute -bottom-6 -right-6 size-32 rounded-full text-cream/[0.12] [mask-image:radial-gradient(closest-side,black,transparent)]" />
                </div>
                <Tape className="-top-3 left-1/2 -ml-11" style={{ transform: `rotate(${tapeAngle[i]}deg)` }} />
                {i % 2 === 0 ? (
                  <Crown className="absolute right-5 top-7 w-9 text-cream/55 transition-colors duration-300 group-hover:text-spray" />
                ) : (
                  <Sparkle className="absolute right-6 top-7 w-8 text-cream/55 transition-colors duration-300 group-hover:text-spray" />
                )}
                <span className="piece relative mt-3 text-[5.4rem]" aria-hidden="true">
                  {s.n}
                </span>
                <h3 className="shout relative mt-6 text-[2rem]">
                  <span className="sr-only">Step {s.n}: </span>
                  {s.title}
                </h3>
                <p className="relative mt-3 text-[0.92rem] leading-relaxed text-dust">{s.body}</p>
                <dl className="relative mt-auto flex flex-wrap gap-2 pt-8">
                  {s.stats.map((st, j) => (
                    <div
                      key={st.k}
                      className={`caps flex items-baseline gap-1.5 rounded-[3px] bg-cream px-2.5 py-1.5 text-ink shadow-[2px_2px_0_#8b5cf6] ${j % 2 ? "rotate-[2deg]" : "-rotate-[2deg]"}`}
                    >
                      <dt className="text-[0.7rem]">{st.k}</dt>
                      <dd className="text-[0.82rem] text-violet-deep">{st.v}</dd>
                    </div>
                  ))}
                </dl>
                <Drips className="absolute inset-x-6 top-full h-5 text-violet/70" />
              </div>
            </m.li>
          ))}
        </ol>
      </div>
    </section>
  );
}
