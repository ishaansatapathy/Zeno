"use client";

import { m } from "motion/react";

import { proof } from "@/content/site";

import { Crown } from "./doodles";
import { Spray } from "./Spray";

const slam = { type: "spring", stiffness: 260, damping: 17 } as const;

const tileTone = {
  cream: "bg-cream text-ink shadow-hard",
  violet: "bg-violet text-ink",
  outline: "border-2 border-cream/50 bg-ink/60 text-violet-hi",
} as const;
const tileTilt = [-2.5, 1.2, 3.5];
const tileSize = [
  "text-[clamp(5rem,9vw,8.5rem)]",
  "text-[clamp(5rem,9vw,8.5rem)]",
  "text-[clamp(4rem,6.5vw,6rem)]",
];

/** A ring of text turning slowly around the number it qualifies. */
function Seal() {
  return (
    <div className="relative grid size-40 shrink-0 place-items-center rounded-full border-2 border-cream/70 lg:size-48">
      <svg viewBox="0 0 200 200" aria-hidden="true" focusable="false" className="absolute inset-0 size-full animate-spin-slow text-violet-hi">
        <defs>
          <path id="seal-ring" d="M100 100m-78 0a78 78 0 1 1 156 0a78 78 0 1 1-156 0" />
        </defs>
        <text className="caps" fontSize="15" letterSpacing="3.2" fill="currentColor">
          <textPath href="#seal-ring">{proof.seal.ring.repeat(2)}</textPath>
        </text>
      </svg>
      <span className="shout text-[4.6rem] leading-none lg:text-[5.4rem]" aria-hidden="true">
        {proof.seal.big}
      </span>
      <span className="sr-only">
        {proof.seal.big} {proof.seal.ring.split(" · ")[0]}, {proof.seal.ring.split(" · ")[1]}
      </span>
    </div>
  );
}

export function Proof() {
  return (
    <section id="proof" aria-labelledby="proof-title" className="relative mx-auto max-w-page scroll-mt-16 px-4 py-24 sm:px-6 lg:px-8 lg:py-32">
      <h2 id="proof-title" className="shout text-[clamp(2.8rem,6.6vw,6.6rem)] lg:whitespace-nowrap">
        {proof.lead} <Spray delay={0.4} underline tone="text-violet-hi">{proof.sprayed}</Spray>
      </h2>

      <div className="relative mt-16 flex flex-wrap items-end gap-x-6 gap-y-8 lg:mt-24 lg:gap-x-8">
        <Crown className="absolute -top-14 left-[44%] hidden w-14 rotate-[6deg] text-cream/60 lg:block" />
        {proof.tiles.map((t, i) => (
          <m.div
            key={t.k}
            initial={{ opacity: 0, scale: 1.35, rotate: tileTilt[i] + 10 }}
            whileInView={{ opacity: 1, scale: 1, rotate: tileTilt[i] }}
            viewport={{ once: true, margin: "0px 0px -12% 0px" }}
            transition={{ ...slam, delay: i * 0.14 }}
            className={`rounded-[6px] px-6 pb-4 pt-5 lg:px-8 ${tileTone[t.tone]}`}
          >
            <p className="caps text-[0.78rem] tracking-[0.12em]">{t.k}</p>
            <p className={`shout mt-1 leading-[0.85] ${tileSize[i]}`}>
              {t.v}
              {t.suffix}
            </p>
          </m.div>
        ))}
        <m.div
          initial={{ opacity: 0, scale: 0.6, rotate: -40 }}
          whileInView={{ opacity: 1, scale: 1, rotate: 0 }}
          viewport={{ once: true, margin: "0px 0px -12% 0px" }}
          transition={{ ...slam, delay: 0.5 }}
          className="lg:absolute lg:-top-20 lg:right-0"
        >
          <Seal />
        </m.div>
      </div>

      <ol className="mt-20 border-t border-cream/15 lg:mt-24">
        {proof.rows.map((r, i) => (
          <li key={r.at} className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-5 gap-y-1 border-b border-cream/15 py-5 lg:grid-cols-[7rem_minmax(0,1fr)_6rem_10rem] lg:py-6">
            <span className="caps text-[0.82rem] text-violet-hi lg:text-[0.9rem]">{r.at}</span>
            <span className="caps col-start-1 text-[1.05rem] text-cream lg:col-start-auto lg:text-[1.3rem]">{r.what}</span>
            <span className="caps hidden text-[0.75rem] tracking-[0.14em] text-dust lg:block">{r.ref}</span>
            <m.span
              initial={{ opacity: 0, scale: 2.4, rotate: -16 }}
              whileInView={{ opacity: 1, scale: 1, rotate: -6 }}
              viewport={{ once: true, margin: "0px 0px -18% 0px" }}
              transition={{ ...slam, delay: 0.15 + i * 0.08 }}
              className="shout col-start-2 row-span-2 row-start-1 justify-self-end whitespace-nowrap rounded-[4px] border-2 border-violet-hi px-3 py-1.5 text-[1rem] text-violet-hi shadow-[0_0_24px_rgba(139,92,246,0.35)] lg:col-start-auto lg:row-span-1 lg:row-start-auto lg:text-[1.25rem]"
            >
              {r.stamp}
            </m.span>
          </li>
        ))}
      </ol>
    </section>
  );
}
