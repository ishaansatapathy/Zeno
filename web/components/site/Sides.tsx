"use client";

import { useState } from "react";

import { sides } from "@/content/site";

import { Button, Container, SectionHead } from "./frame";
import { Scramble } from "./Scramble";

/**
 * Two panels side by side, one for each kind of person. Point at one and it comes forward, its
 * name settling out of noise, while the other steps back.
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
    <section id="sides" aria-labelledby="sides-title" className="scroll-mt-14 border-t border-line py-28 lg:py-36">
      <Container>
        <SectionHead id="sides-title" n="02" label={sides.label} title={sides.title} aside={sides.aside} />
        <div onPointerLeave={() => setHot(null)} className="mt-16 grid gap-4 lg:mt-20 lg:grid-cols-2">
          {sides.panels.map((p, i) => {
            const on = hot === i;
            const dim = hot !== null && !on;
            return (
              <div
                key={p.tag}
                onPointerEnter={() => light(i)}
                onFocus={() => light(i)}
                className={`relative flex min-h-[22rem] flex-col overflow-hidden rounded-xl border bg-panel/80 p-7 transition-[opacity,border-color] duration-500 sm:p-9 lg:min-h-[26rem] ${on ? "border-line-hi" : "border-line"} ${dim ? "opacity-40" : ""}`}
              >
                <div aria-hidden="true" className={`pointer-events-none absolute inset-0 bg-[radial-gradient(ellipse_70%_60%_at_20%_100%,rgba(180,194,214,0.09),transparent_70%)] transition-opacity duration-500 ${on ? "opacity-100" : "opacity-0"}`} />
                <div className="relative flex items-center justify-between">
                  <p className="label flex items-center gap-2">
                    <span className={`size-1.5 rounded-full transition-colors ${on ? "bg-frost" : "bg-line-hi"}`} />
                    {p.tag}
                  </p>
                  <p className="label">0{i + 1}</p>
                </div>
                <h3 className="display relative mt-auto text-[clamp(2.8rem,5.4vw,4.8rem)]">
                  <Scramble text={p.word} replay={replays[i]} duration={500} />
                </h3>
                <p className="relative mt-4 text-[1.05rem] leading-snug">
                  <span className="block">{p.line[0]}</span>
                  <span className="block text-mute">{p.line[1]}</span>
                </p>
                <div className="relative mt-8">
                  <Button href={p.cta.href} tone={on ? "solid" : "line"}>
                    {p.cta.label}
                  </Button>
                </div>
              </div>
            );
          })}
        </div>
        <div className="mt-6 flex flex-wrap justify-between gap-3 px-1">
          <p className="label flex items-center gap-2">
            <span className="size-1.5 rounded-full bg-frost" />
            {sides.foot.left}
          </p>
          <p className="label">{sides.foot.right}</p>
        </div>
      </Container>
    </section>
  );
}
