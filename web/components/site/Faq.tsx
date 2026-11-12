"use client";

import { useId, useState } from "react";

import { faq } from "@/content/site";

import { Label } from "./frame";

function Item({ q, a, open, onToggle }: { q: string; a: string; open: boolean; onToggle: () => void }) {
  const id = useId();
  return (
    <li className="border-b border-line">
      <h3>
        <button
          type="button"
          aria-expanded={open}
          aria-controls={id}
          onClick={onToggle}
          className="group flex w-full items-center justify-between gap-6 px-1 py-6 text-left transition-colors hover:text-signal"
        >
          <span className="display text-[clamp(1.15rem,1.7vw,1.45rem)]">{q}</span>
          <span aria-hidden="true" className={`font-mono text-lg transition-transform duration-300 ${open ? "rotate-45 text-signal" : "text-mute"}`}>
            +
          </span>
        </button>
      </h3>
      <div id={id} className={`grid transition-[grid-template-rows] duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] ${open ? "grid-rows-[1fr]" : "grid-rows-[0fr]"}`}>
        <div className="overflow-hidden" inert={!open}>
          <p className="max-w-[38rem] px-1 pb-7 text-[0.95rem] leading-relaxed text-mute">{a}</p>
        </div>
      </div>
    </li>
  );
}

/** One giant word on the left, the questions on the right. */
export function Faq() {
  const [open, setOpen] = useState<number | null>(0);
  return (
    <section id="faq" aria-labelledby="faq-title" className="mx-auto max-w-page scroll-mt-12 px-4 py-28 sm:px-6 lg:px-10 lg:py-36">
      <div className="grid border-t border-line lg:grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)]">
        <div className="border-line pb-10 pt-8 lg:border-r lg:pr-12">
          <Label>{faq.label}</Label>
          <h2 id="faq-title" className="mt-6 font-cond text-[clamp(7rem,17vw,15rem)] uppercase leading-[0.8] tracking-[-0.01em]">
            {faq.word}
          </h2>
          <p className="mt-8 max-w-[22rem] text-[0.95rem] leading-relaxed text-mute">{faq.aside}</p>
        </div>
        <ul className="lg:pl-12">
          {faq.items.map((it, i) => (
            <Item key={it.q} q={it.q} a={it.a} open={open === i} onToggle={() => setOpen(open === i ? null : i)} />
          ))}
        </ul>
      </div>
    </section>
  );
}
