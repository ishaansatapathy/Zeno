"use client";

import { useState } from "react";

import { promises } from "@/content/site";

import { SectionHead } from "./frame";

/**
 * The promises as a table. The row being looked at fills with orange from the bottom up and
 * opens to say how the promise is kept; the first is open until another is chosen.
 */
export function Promises() {
  const [active, setActive] = useState(0);
  return (
    <section id="promises" aria-labelledby="promises-title" className="mx-auto max-w-page scroll-mt-12 px-4 py-28 sm:px-6 lg:px-10 lg:py-36">
      <SectionHead id="promises-title" label={promises.label} title={promises.title} aside={promises.aside} />
      <ol className="mt-16 border-t border-line lg:mt-20">
        {promises.rows.map((r, i) => {
          const on = i === active;
          return (
            <li
              key={r.n}
              tabIndex={0}
              onPointerEnter={() => setActive(i)}
              onFocus={() => setActive(i)}
              onClick={() => setActive(i)}
              data-on={on}
              className="group relative isolate grid cursor-default grid-cols-[2.5rem_minmax(0,1fr)_auto] items-start gap-x-4 gap-y-3 border-b border-line px-3 py-6 outline-offset-[-2px] sm:px-5 lg:grid-cols-[3.5rem_10rem_15rem_minmax(0,1fr)_9rem] lg:items-center lg:gap-x-6 lg:py-7"
            >
              <span aria-hidden="true" className={`absolute inset-0 -z-10 origin-bottom bg-signal transition-transform duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] ${on ? "scale-y-100" : "scale-y-0"}`} />
              <span className="label pt-1 group-data-[on=true]:text-ink/70 lg:pt-0">{r.n}</span>
              <span className="lg:block">
                <span className="label block group-data-[on=true]:text-ink/70">{r.tag}</span>
                <span className="display mt-1 block text-[1.6rem] italic transition-colors group-data-[on=true]:text-ink">{r.word}</span>
              </span>
              <span className="display col-start-2 text-[1.2rem] leading-snug transition-colors group-data-[on=true]:text-ink lg:col-start-auto">{r.title}</span>
              <span className="col-start-2 text-[0.9rem] leading-relaxed text-mute transition-colors group-data-[on=true]:text-ink/80 lg:col-start-auto">
                {r.body}
                <span className={`grid transition-[grid-template-rows] duration-500 ${on ? "grid-rows-[1fr]" : "grid-rows-[0fr]"}`}>
                  <span className="overflow-hidden">
                    <span className="mt-2 block font-mono text-[0.72rem] uppercase tracking-[0.08em]">{r.more}</span>
                  </span>
                </span>
              </span>
              <span className="col-start-3 row-span-2 row-start-1 text-right lg:col-start-auto lg:row-span-1 lg:row-start-auto">
                <span className="display block text-[clamp(2.2rem,3.6vw,3.2rem)] leading-none transition-colors group-data-[on=true]:text-ink">{r.big}</span>
                <span className="label mt-1 block group-data-[on=true]:text-ink/70">{r.unit}</span>
              </span>
            </li>
          );
        })}
      </ol>
    </section>
  );
}
