"use client";

import { useState } from "react";

import { promises } from "@/content/site";

import { Container, SectionHead } from "./frame";

/**
 * The promises as a table. The row being looked at lifts a shade, gains a frost rule at its left
 * edge and opens to say how the promise is kept; the first is open until another is chosen.
 */
export function Promises() {
  const [active, setActive] = useState(0);
  return (
    <section id="promises" aria-labelledby="promises-title" className="scroll-mt-14 border-t border-line py-28 lg:py-36">
      <Container>
        <SectionHead id="promises-title" n="05" label={promises.label} title={promises.title} aside={promises.aside} />
        <ol className="mt-16 overflow-hidden rounded-xl border border-line lg:mt-20">
          {promises.rows.map((r, i) => {
            const on = i === active;
            return (
              <li
                key={r.n}
                tabIndex={0}
                onPointerEnter={() => setActive(i)}
                onFocus={() => setActive(i)}
                onClick={() => setActive(i)}
                className={`relative grid cursor-default grid-cols-[2.5rem_minmax(0,1fr)_auto] items-start gap-x-4 gap-y-2 px-5 py-6 outline-offset-[-2px] transition-colors duration-300 sm:px-6 lg:grid-cols-[3rem_11rem_15rem_minmax(0,1fr)_8rem] lg:items-center lg:gap-x-6 ${i ? "border-t border-line" : ""} ${on ? "bg-raise" : "bg-panel/60"}`}
              >
                <span aria-hidden="true" className={`absolute inset-y-0 left-0 w-px origin-top bg-frost transition-transform duration-500 ${on ? "scale-y-100" : "scale-y-0"}`} />
                <span className="font-mono text-[0.72rem] text-mute lg:pt-0">{r.n}</span>
                <span>
                  <span className="label block">{r.tag}</span>
                  <span className={`mt-1 block text-[1.15rem] font-medium tracking-[-0.01em] transition-colors ${on ? "text-paper" : "text-paper/80"}`}>{r.word}</span>
                </span>
                <span className="col-start-2 text-[0.98rem] font-medium leading-snug lg:col-start-auto">{r.title}</span>
                <span className="col-start-2 text-[0.9rem] leading-relaxed text-mute lg:col-start-auto">
                  {r.body}
                  <span className={`grid transition-[grid-template-rows] duration-500 ${on ? "grid-rows-[1fr]" : "grid-rows-[0fr]"}`}>
                    <span className="overflow-hidden">
                      <span className="mt-2 block text-[0.85rem] text-frost">{r.more}</span>
                    </span>
                  </span>
                </span>
                <span className="col-start-3 row-span-2 row-start-1 text-right lg:col-start-auto lg:row-span-1 lg:row-start-auto">
                  <span className={`display block text-[clamp(2rem,3.2vw,2.8rem)] leading-none transition-colors ${on ? "text-paper" : "text-mute"}`}>{r.big}</span>
                  <span className="label mt-1.5 block">{r.unit}</span>
                </span>
              </li>
            );
          })}
        </ol>
      </Container>
    </section>
  );
}
