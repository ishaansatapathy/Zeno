"use client";

import Link from "next/link";
import { useState } from "react";

import { sides } from "@/content/site";

import { ArrowRight, ArrowUpRight, Container, Crosses, SectionHead } from "./frame";
import { Scramble } from "./Scramble";

/**
 * Two panels in one frame, one for each kind of person. Point at one and it lights up, its name
 * settling out of noise in polished silver, while the other dims.
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
    <section id="sides" aria-labelledby="sides-title" className="scroll-mt-16 py-28 lg:py-36">
      <Container>
        <SectionHead id="sides-title" n="02" label={sides.label} title={sides.title} aside={sides.aside} />
        <div onPointerLeave={() => setHot(null)} className="relative mt-16 border border-line bg-ink/60 lg:mt-20">
          <Crosses />
          <div className="grid lg:grid-cols-2">
            {sides.panels.map((p, i) => {
              const on = hot === i;
              const dim = hot !== null && !on;
              const cta = `caps mt-10 inline-flex w-fit items-center gap-3 border-b pb-1.5 transition-colors ${on ? "border-silver text-silver-hi" : "border-paper/30 text-paper/85"}`;
              return (
                <div
                  key={p.tag}
                  onPointerEnter={() => light(i)}
                  onFocus={() => light(i)}
                  className={`relative flex min-h-[24rem] flex-col p-7 transition-[opacity,background-color] duration-500 sm:p-10 lg:min-h-[29rem] lg:p-12 ${i ? "border-t border-line lg:border-l lg:border-t-0" : ""} ${dim ? "opacity-35" : ""} ${on ? "bg-[radial-gradient(ellipse_at_30%_65%,rgba(200,205,212,0.08),transparent_65%)]" : ""}`}
                >
                  <div className="flex items-start justify-between">
                    <span aria-hidden="true" className="font-mono text-mute">
                      +
                    </span>
                    <p className={`label flex items-center gap-2 ${on ? "text-paper" : ""}`}>
                      <span aria-hidden="true" className={`size-1.5 ${on ? "bg-silver" : "bg-line-hi"}`} />
                      {p.tag}
                    </p>
                  </div>
                  <h3 className="display mt-auto text-[clamp(3.6rem,7.6vw,7rem)]">
                    <em className={on ? "metal" : "pr-[0.08em] font-serif italic text-paper"}>
                      <Scramble text={p.word} replay={replays[i]} />
                    </em>
                  </h3>
                  <p className="mt-4 text-[1.1rem] leading-snug">
                    <span className="block">{p.line[0]}</span>
                    <span className="block text-mute">{p.line[1]}</span>
                  </p>
                  {p.cta.external ? (
                    <Link href={p.cta.href} prefetch={false} className={cta}>
                      {p.cta.label}
                      <ArrowUpRight />
                    </Link>
                  ) : (
                    <a href={p.cta.href} className={cta}>
                      {p.cta.label}
                      <ArrowRight />
                    </a>
                  )}
                </div>
              );
            })}
          </div>
          <div className="flex flex-wrap justify-between gap-3 border-t border-line px-6 py-4 sm:px-10">
            <p className="label flex items-center gap-2 text-silver">
              <span aria-hidden="true" className="size-1.5 bg-silver" />
              {sides.foot.left}
            </p>
            <p className="label">{sides.foot.right}</p>
          </div>
        </div>
      </Container>
    </section>
  );
}
