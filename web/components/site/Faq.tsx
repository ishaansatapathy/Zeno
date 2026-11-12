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
          className="flex w-full items-center justify-between gap-6 py-6 text-left text-[1.05rem] font-medium tracking-[-0.01em] transition-colors hover:text-white"
        >
          {q}
          <span aria-hidden="true" className="relative size-3 shrink-0 text-mute">
            <span className="absolute inset-x-0 top-1/2 h-px -translate-y-1/2 bg-current" />
            <span className={`absolute inset-y-0 left-1/2 w-px -translate-x-1/2 bg-current transition-transform duration-300 ${open ? "scale-y-0" : ""}`} />
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
    <section id="faq" aria-labelledby="faq-title" className="scroll-mt-14 border-t border-line py-28 lg:py-36">
      <Container className="grid gap-12 lg:grid-cols-[minmax(0,0.8fr)_minmax(0,1.2fr)] lg:gap-20">
        <div>
          <Label n="06">{faq.label}</Label>
          <Title id="faq-title" text={faq.title} className="mt-6 text-[clamp(2.4rem,5vw,4.6rem)]" />
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
