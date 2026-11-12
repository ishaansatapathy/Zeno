"use client";

import { useState, type ReactNode } from "react";

import { pillars } from "@/content/site";

import { Container, Crosses, SectionHead } from "./frame";
import { Scramble } from "./Scramble";

const icons: ReactNode[] = [
  // Stream: five bars of rising quality.
  <>
    <path d="M4 18v-3M8 18v-6M12 18V9M16 18V6M20 18V3" />
  </>,
  // Lock.
  <>
    <rect x="5" y="10.5" width="14" height="10" />
    <path d="M8.5 10.5V7.5a3.5 3.5 0 0 1 7 0v3" />
  </>,
  // Count: a ledger's two columns.
  <>
    <rect x="3.5" y="4" width="17" height="16" />
    <path d="M12 4v16M6.5 9h3M14.5 9h3M6.5 13h3M14.5 13h3" />
  </>,
];

/**
 * The three parts under every lesson, as three columns. The one being looked at fills with silver
 * from the bottom up, its name re-settling out of noise.
 */
export function Pillars() {
  const [hot, setHot] = useState<number | null>(null);
  const [replays, setReplays] = useState([0, 0, 0]);

  function light(i: number) {
    if (hot === i) return;
    setHot(i);
    setReplays((r) => r.map((n, j) => (j === i ? n + 1 : n)));
  }

  return (
    <section aria-labelledby="pillars-title" className="py-28 lg:py-36">
      <Container>
        <SectionHead id="pillars-title" n="04" label={pillars.label} title={pillars.title} aside={pillars.aside} />
        <div onPointerLeave={() => setHot(null)} className="relative mt-16 grid border border-line bg-ink/60 lg:mt-20 lg:grid-cols-3">
          <Crosses />
          {pillars.items.map((it, i) => {
            const on = hot === i;
            return (
              <div
                key={it.word}
                tabIndex={0}
                onPointerEnter={() => light(i)}
                onFocus={() => light(i)}
                className={`relative isolate flex min-h-[22rem] flex-col overflow-hidden p-7 outline-offset-[-2px] transition-colors duration-500 sm:p-9 lg:min-h-[27rem] ${i ? "border-t border-line lg:border-l lg:border-t-0" : ""} ${on ? "text-ink" : ""}`}
              >
                <span aria-hidden="true" className={`absolute inset-0 -z-10 origin-bottom bg-silver transition-transform duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] ${on ? "scale-y-100" : "scale-y-0"}`} />
                <p className={`label transition-colors ${on ? "text-ink/70" : ""}`}>{it.n}</p>
                <svg viewBox="0 0 24 24" aria-hidden="true" focusable="false" className={`mt-8 size-9 fill-none stroke-current transition-colors ${on ? "text-ink" : "text-silver"}`} strokeWidth="1.4">
                  {icons[i]}
                </svg>
                <h3 className="display mt-auto text-[clamp(3rem,5vw,4.6rem)]">
                  <em className={on ? "pr-[0.08em] font-serif italic" : "metal"}>
                    <Scramble text={it.word} replay={replays[i]} />
                  </em>
                </h3>
                <p className={`label mt-3 transition-colors ${on ? "text-ink/70" : ""}`}>{it.name}</p>
                <p className={`mt-4 max-w-[22rem] text-[0.92rem] leading-relaxed transition-colors ${on ? "text-ink/80" : "text-mute"}`}>{it.body}</p>
              </div>
            );
          })}
        </div>
      </Container>
    </section>
  );
}
