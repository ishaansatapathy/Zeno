"use client";

import Link from "next/link";
import { useId, useState } from "react";

import { cn } from "@/components/ui/cn";

export type Segment = { key: string; label: string; line: string; points: string[]; cta: string; href: string };

/** The four tiny registration marks a printed card has at its corners. */
function Marks({ dark }: { dark: boolean }) {
  const dot = cn("absolute h-[3px] w-[3px] rounded-full", dark ? "bg-black/40" : "bg-white/30");
  return (
    <span aria-hidden="true">
      <span className={cn(dot, "left-2 top-2")} />
      <span className={cn(dot, "right-2 top-2")} />
      <span className={cn(dot, "bottom-2 left-2")} />
      <span className={cn(dot, "bottom-2 right-2")} />
    </span>
  );
}

/**
 * A stack of cards with one open at a time. The open one turns to polished silver and unfolds its
 * list; the others stay dark, showing only their name. Heights animate through grid rows, so the
 * open card grows from nothing to its natural size without measuring anything.
 */
export function SegmentStack({ segments }: { segments: Segment[] }) {
  const base = useId();
  const [open, setOpen] = useState(0);
  return (
    <div className="flex flex-col gap-2">
      {segments.map((segment, i) => {
        const on = i === open;
        const panel = `${base}-${segment.key}`;
        return (
          <div
            key={segment.key}
            className={cn("relative overflow-hidden rounded-card transition-[background,color,box-shadow] duration-500", on ? "plate" : "metal")}
          >
            <Marks dark={on} />
            <button
              type="button"
              aria-expanded={on}
              aria-controls={panel}
              onClick={() => setOpen(i)}
              onMouseEnter={() => setOpen(i)}
              className="flex w-full items-start justify-between gap-6 px-6 pb-4 pt-5 text-left"
            >
              <span className="shout text-[clamp(2.6rem,5vw,4rem)]">{segment.label}</span>
              <span className={cn("mt-2 flex items-start gap-4 text-[0.92rem] leading-snug", on ? "text-black/70" : "text-text-muted")}>
                <span className="hidden max-w-[11rem] sm:block">{segment.line}</span>
                <span aria-hidden="true" className="font-mono text-lg leading-none">
                  {on ? "−" : "+"}
                </span>
              </span>
            </button>
            <div
              id={panel}
              className={cn("grid transition-[grid-template-rows] duration-500 ease-[cubic-bezier(0.22,1,0.36,1)]", on ? "grid-rows-[1fr]" : "grid-rows-[0fr]")}
            >
              <div className="overflow-hidden">
                <div className="flex flex-col gap-6 px-6 pb-6 pt-6">
                  <ul className="flex flex-col gap-2">
                    {segment.points.map((point) => (
                      <li key={point} className="flex items-center gap-3 text-[0.95rem]">
                        <svg viewBox="0 0 12 12" aria-hidden="true" className="h-3 w-3 shrink-0">
                          <path d="M2 6.5 5 9.5 10 3" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />
                        </svg>
                        {point}
                      </li>
                    ))}
                  </ul>
                  <Link
                    href={segment.href}
                    tabIndex={on ? 0 : -1}
                    className="inline-flex h-11 w-fit items-center gap-2 rounded-full bg-[#050505] px-5 text-[0.9rem] text-[#f4f4f5] transition-colors hover:bg-[#1c1c1f]"
                  >
                    {segment.cta}
                  </Link>
                </div>
              </div>
            </div>
          </div>
        );
      })}
    </div>
  );
}
