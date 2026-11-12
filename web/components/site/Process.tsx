"use client";

import { m } from "motion/react";

import { process } from "@/content/site";

import { Crown } from "./doodles";
import { Spray } from "./Spray";

const ease = [0.22, 1, 0.36, 1] as const;

/** Each card lands at its own slight angle, like four cards dropped on a table. */
const tilt = [-2.2, 1.6, -1.2, 2.4];
const drop = ["lg:mt-0", "lg:mt-7", "lg:mt-2", "lg:mt-9"];

export function Process() {
  return (
    <section id="how" aria-labelledby="how-title" className="relative mx-auto max-w-page scroll-mt-16 px-4 py-24 sm:px-6 lg:px-8 lg:py-32">
      <h2 id="how-title" className="shout text-[clamp(3rem,8.4vw,8.6rem)]">
        <span className="block">
          {process.lead}{" "}
          <span className="relative inline-block text-cream/25">
            {process.struck}
            <svg viewBox="0 0 300 30" preserveAspectRatio="none" aria-hidden="true" focusable="false" className="absolute left-[-3%] top-[46%] h-[0.2em] w-[106%] text-violet">
              <m.path
                d="M4 20C70 12 150 9 296 6"
                fill="none"
                stroke="currentColor"
                strokeWidth={9}
                strokeLinecap="round"
                initial={{ pathLength: 0 }}
                whileInView={{ pathLength: 1 }}
                viewport={{ once: true, margin: "0px 0px -20% 0px" }}
                transition={{ duration: 0.7, delay: 0.2, ease }}
              />
            </svg>
          </span>
        </span>
        <span className="block">
          {process.mid} <Spray delay={0.7}>{process.sprayed}</Spray>
        </span>
      </h2>

      <ol className="mt-16 grid gap-6 sm:grid-cols-2 lg:mt-20 lg:grid-cols-4 lg:gap-5">
        {process.steps.map((s, i) => (
          <m.li
            key={s.n}
            className={drop[i]}
            initial={{ opacity: 0, y: 70, rotate: 0 }}
            whileInView={{ opacity: 1, y: 0, rotate: tilt[i] }}
            viewport={{ once: true, margin: "0px 0px -10% 0px" }}
            transition={{ duration: 0.9, delay: i * 0.1, ease }}
          >
            <div className="slab group relative flex h-full min-h-[23rem] flex-col p-6 pb-5 transition-transform duration-500 ease-out hover:-translate-y-2 hover:rotate-[calc(var(--r)*-1)]" style={{ ["--r" as string]: `${tilt[i]}deg` }}>
              <Crown className="absolute right-5 top-6 w-9 text-cream/55 transition-colors duration-300 group-hover:text-spray" />
              <span className="shout text-[5.2rem] leading-[0.8] text-violet" aria-hidden="true">
                {s.n}
              </span>
              <h3 className="shout mt-7 text-[2rem]">
                <span className="sr-only">Step {s.n}: </span>
                {s.title}
              </h3>
              <p className="mt-3 text-[0.92rem] leading-relaxed text-dust">{s.body}</p>
              <dl className="mt-auto flex gap-5 rounded-md border border-cream/15 bg-ink/60 px-3.5 py-2.5">
                {s.stats.map((st) => (
                  <div key={st.k} className="flex items-baseline gap-1.5">
                    <dt className="caps text-[0.72rem] text-cream">{st.k}</dt>
                    <dd className="caps text-[0.78rem] text-violet-hi">{st.v}</dd>
                  </div>
                ))}
              </dl>
            </div>
          </m.li>
        ))}
      </ol>
    </section>
  );
}
