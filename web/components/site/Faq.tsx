"use client";

import { useId, useState } from "react";

import { faq } from "@/content/site";

import { Container, Label, Title } from "./frame";

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
          className="flex w-full items-center justify-between gap-6 py-6 text-left transition-colors hover:text-silver-hi"
        >
          <span className="display text-[clamp(1.35rem,1.9vw,1.7rem)]">{q}</span>
          <span aria-hidden="true" className={`font-mono text-lg transition-transform duration-300 ${open ? "rotate-45 text-silver" : "text-mute"}`}>
            +
          </span>
        </button>
      </h3>
      <div id={id} className={`grid transition-[grid-template-rows] duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] ${open ? "grid-rows-[1fr]" : "grid-rows-[0fr]"}`}>
        <div className="overflow-hidden" inert={!open}>
          <p className="max-w-[38rem] pb-7 text-[0.95rem] leading-relaxed text-mute">{a}</p>
        </div>
      </div>
    </li>
  );
}

/** The title and a note on the left, the questions on the right. */
export function Faq() {
  const [open, setOpen] = useState<number | null>(0);
  return (
    <section id="faq" aria-labelledby="faq-title" className="scroll-mt-16 py-28 lg:py-36">
      <Container className="grid gap-12 lg:grid-cols-[minmax(0,0.85fr)_minmax(0,1.15fr)] lg:gap-20">
        <div>
          <Label n="06">{faq.label}</Label>
          <Title id="faq-title" text={faq.title} className="mt-6 text-[clamp(2.8rem,6vw,5.6rem)]" />
          <p className="mt-6 max-w-[22rem] text-[0.95rem] leading-relaxed text-mute">{faq.aside}</p>
        </div>
        <ul className="border-t border-line">
          {faq.items.map((it, i) => (
            <Item key={it.q} q={it.q} a={it.a} open={open === i} onToggle={() => setOpen(open === i ? null : i)} />
          ))}
        </ul>
      </Container>
    </section>
  );
}
