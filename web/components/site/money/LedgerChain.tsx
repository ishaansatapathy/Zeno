"use client";

import { m } from "motion/react";

/** The four things that happen to a payment, ticked off one after another as the chain is seen. */
export function LedgerChain({ steps }: { steps: string[] }) {
  return (
    <ol className="relative flex flex-col gap-4 pl-10">
      <svg aria-hidden="true" className="absolute left-[11px] top-3 h-[calc(100%-1.5rem)] w-1 overflow-visible">
        <m.line
          x1="1"
          y1="0"
          x2="1"
          y2="100%"
          stroke="rgb(var(--text))"
          strokeWidth="1.5"
          initial={{ pathLength: 0 }}
          whileInView={{ pathLength: 1 }}
          viewport={{ once: true, margin: "0px 0px -15% 0px" }}
          transition={{ duration: 1.6, ease: "easeInOut" }}
        />
      </svg>
      {steps.map((step, i) => (
        <m.li
          key={step}
          className="relative flex items-center gap-3 text-[1.02rem]"
          initial={{ opacity: 0.25 }}
          whileInView={{ opacity: 1 }}
          viewport={{ once: true, margin: "0px 0px -15% 0px" }}
          transition={{ duration: 0.3, delay: 0.25 + i * 0.4 }}
        >
          <m.span
            aria-hidden="true"
            className="absolute -left-10 grid h-6 w-6 place-items-center rounded-full border border-text bg-surface"
            initial={{ backgroundColor: "rgb(var(--surface))" }}
            whileInView={{ backgroundColor: i === steps.length - 1 ? "rgb(var(--success))" : "rgb(var(--accent))" }}
            viewport={{ once: true, margin: "0px 0px -15% 0px" }}
            transition={{ duration: 0.3, delay: 0.25 + i * 0.4 }}
          >
            <svg viewBox="0 0 12 12" className="h-3 w-3">
              <path
                d="M2 6.5 5 9.5 10 3"
                fill="none"
                stroke={i === steps.length - 1 ? "rgb(var(--surface))" : "rgb(var(--text))"}
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          </m.span>
          {step}
        </m.li>
      ))}
    </ol>
  );
}
