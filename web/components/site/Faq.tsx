"use client";

import { useId, useState } from "react";

import { faq } from "@/content/site";

import { Spray } from "./Spray";

function Item({ q, a, open, onToggle }: { q: string; a: string; open: boolean; onToggle: () => void }) {
  const id = useId();
  return (
    <li className="border-b border-cream/15">
      <h3>
        <button
          type="button"
          aria-expanded={open}
          aria-controls={id}
          onClick={onToggle}
          className="caps group flex w-full items-center justify-between gap-6 py-5 text-left text-[1.1rem] text-cream transition-colors hover:text-spray lg:text-[1.25rem]"
        >
          {q}
          <svg viewBox="0 0 20 20" aria-hidden="true" focusable="false" className={`size-5 shrink-0 text-violet-hi transition-transform duration-300 ${open ? "rotate-45" : ""}`}>
            <path d="M10 3v14M3 10h14" fill="none" stroke="currentColor" strokeWidth={2.2} strokeLinecap="round" />
          </svg>
        </button>
      </h3>
      <div id={id} className={`grid transition-[grid-template-rows] duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] ${open ? "grid-rows-[1fr]" : "grid-rows-[0fr]"}`}>
        <div className="overflow-hidden" inert={!open}>
          <p className="max-w-[34rem] pb-6 text-[0.95rem] leading-relaxed text-dust">{a}</p>
        </div>
      </div>
    </li>
  );
}

export function Faq() {
  const [open, setOpen] = useState<number | null>(0);
  return (
    <section id="faq" aria-labelledby="faq-title" className="relative mx-auto max-w-page scroll-mt-16 px-4 py-24 sm:px-6 lg:px-8 lg:py-32">
      <span aria-hidden="true" className="shout pointer-events-none absolute -left-4 top-[34%] -z-10 select-none text-[clamp(10rem,28vw,26rem)] text-cream/[0.045]">
        Ask
      </span>
      <div className="grid gap-12 lg:grid-cols-[minmax(0,1fr)_minmax(0,1.15fr)] lg:gap-20">
        <div>
          <h2 id="faq-title" className="shout text-[clamp(3rem,6.6vw,6.6rem)]">
            <span className="block">{faq.lead}</span>
            <span className="block">
              {faq.line2} <Spray delay={0.4} underline>{faq.sprayed}</Spray>
            </span>
          </h2>
          <p className="mt-12 flex max-w-[22rem] gap-3 text-[0.92rem] leading-relaxed text-dust">
            <span aria-hidden="true" className="text-3xl leading-none text-violet-hi">
              [
            </span>
            <span className="pt-1">{faq.note}</span>
            <span aria-hidden="true" className="self-end text-3xl leading-none text-violet-hi">
              ]
            </span>
          </p>
        </div>
        <ul className="border-t border-cream/15 lg:mt-3">
          {faq.items.map((it, i) => (
            <Item key={it.q} q={it.q} a={it.a} open={open === i} onToggle={() => setOpen(open === i ? null : i)} />
          ))}
        </ul>
      </div>
    </section>
  );
}
