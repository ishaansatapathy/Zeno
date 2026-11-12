"use client";

import Link from "next/link";
import { useState } from "react";

import { sides } from "@/content/site";

import { ArrowRight, ArrowUpRight, Crosses, SectionHead } from "./frame";
import { Scramble } from "./Scramble";

/**
 * Two panels side by side, one for each kind of person. Point at one and it lights up, its name
 * re-settling out of noise, while the other steps back.
 */
export function Sides() {
  const [hot, setHot] = useState<number | null>(null);
  const [replays, setReplays] = useState([0, 0]);

  function light(i: number) {
    if (hot === i) return;
    setHot(i);
    setReplays((r) => r.map((n, j) => (j === i ? n + 1 : n)));
  }

  return (
    <section id="sides" aria-labelledby="sides-title" className="mx-auto max-w-page scroll-mt-12 px-4 py-28 sm:px-6 lg:px-10 lg:py-36">
      <SectionHead id="sides-title" label={sides.label} title={sides.title} aside={sides.aside} />
      <div onPointerLeave={() => setHot(null)} className="relative mt-16 border border-line lg:mt-20">
        <Crosses />
        <div className="grid lg:grid-cols-2">
          {sides.panels.map((p, i) => {
            const on = hot === i;
            const dim = hot !== null && !on;
            return (
              <div
                key={p.tag}
                onPointerEnter={() => light(i)}
                onFocus={() => light(i)}
                className={`relative flex min-h-[24rem] flex-col p-7 transition-[opacity,background-color] duration-500 sm:p-10 lg:min-h-[30rem] lg:p-12 ${i ? "border-t border-line lg:border-l lg:border-t-0" : ""} ${dim ? "opacity-35" : ""} ${on ? "bg-[radial-gradient(ellipse_at_30%_60%,rgba(255,91,20,0.10),transparent_65%)]" : ""}`}
              >
                <div className="flex items-start justify-between">
                  <span aria-hidden="true" className="font-mono text-mute">
                    +
                  </span>
                  <p className={`label flex items-center gap-2 ${on ? "text-paper" : ""}`}>
                    {i === 1 ? <span className="size-1.5 bg-signal" /> : null}
                    {p.tag}
                  </p>
                </div>
                <h3 className={`display mt-auto text-[clamp(3.4rem,7.4vw,6.6rem)] italic transition-colors duration-500 ${on ? "text-signal" : "text-paper"}`}>
                  <Scramble text={p.word} replay={replays[i]} />
                </h3>
                <p className="display mt-4 text-[clamp(1.15rem,1.6vw,1.45rem)] leading-snug">
                  <span className="block">{p.line[0]}</span>
                  <span className="block text-paper/60">{p.line[1]}</span>
                </p>
                {p.cta.external ? (
                  <Link href={p.cta.href} prefetch={false} className={`caps mt-10 inline-flex w-fit items-center gap-3 border-b pb-1.5 transition-colors ${on ? "border-signal text-signal" : "border-paper/40"}`}>
                    {p.cta.label}
                    <ArrowUpRight />
                  </Link>
                ) : (
                  <a href={p.cta.href} className={`caps mt-10 inline-flex w-fit items-center gap-3 border-b pb-1.5 transition-colors ${on ? "border-signal text-signal" : "border-paper/40"}`}>
                    {p.cta.label}
                    <ArrowRight />
                  </a>
                )}
              </div>
            );
          })}
        </div>
        <div className="flex flex-wrap justify-between gap-3 border-t border-line px-6 py-4 sm:px-10">
          <p className="label flex items-center gap-2 text-signal">
            <span className="size-1.5 bg-signal" />
            {sides.foot.left}
          </p>
          <p className="label">{sides.foot.right}</p>
        </div>
      </div>
    </section>
  );
}
