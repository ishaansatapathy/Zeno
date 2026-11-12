"use client";

import { useLenis } from "lenis/react";
import { animate, AnimatePresence, m, useInView, useMotionValue, useMotionValueEvent, useReducedMotion, useScroll, useTransform, type MotionValue } from "motion/react";
import { useEffect, useRef, useState, type ComponentType } from "react";

import { cn } from "@/components/ui/cn";

import { useMedia } from "../useMedia";

import { DeliverDemo, ProtectDemo, TranscodeDemo, UploadDemo } from "./demos";

export type Step = { key: "upload" | "transcode" | "protect" | "deliver"; title: string; body: string };

const DEMOS: Record<Step["key"], ComponentType<{ p: MotionValue<number> }>> = {
  upload: UploadDemo,
  transcode: TranscodeDemo,
  protect: ProtectDemo,
  deliver: DeliverDemo,
};

const clamp = (v: number) => Math.min(1, Math.max(0, v));

function Stage({ step, index, total, p }: { step: Step; index: number; total: number; p: MotionValue<number> }) {
  const Demo = DEMOS[step.key];
  return (
    <div className="overflow-hidden rounded-[1.5rem] border border-text bg-surface shadow-[8px_8px_0_0_rgb(var(--text))]">
      <div className="flex items-center justify-between border-b border-text px-6 py-3">
        <p className="label">
          {String(index + 1).padStart(2, "0")} / {String(total).padStart(2, "0")}
        </p>
        <p className="label text-text">{step.title}</p>
      </div>
      <div className="dot-grid min-h-[18rem] p-5 sm:p-8 lg:h-[25rem]">
        <Demo p={p} />
      </div>
    </div>
  );
}

/** Desktop: the section holds still while the page scrolls, and the scroll itself runs the demos. */
function Pinned({ steps }: { steps: Step[] }) {
  const ref = useRef<HTMLDivElement>(null);
  const lenis = useLenis();
  const { scrollYProgress } = useScroll({ target: ref, offset: ["start start", "end end"] });
  const n = steps.length;
  const p0 = useTransform(scrollYProgress, (v) => clamp(v * n - 0));
  const p1 = useTransform(scrollYProgress, (v) => clamp(v * n - 1));
  const p2 = useTransform(scrollYProgress, (v) => clamp(v * n - 2));
  const p3 = useTransform(scrollYProgress, (v) => clamp(v * n - 3));
  const locals = [p0, p1, p2, p3];
  const fill = useTransform(scrollYProgress, [0, 1], ["0%", "100%"]);
  const [active, setActive] = useState(0);
  useMotionValueEvent(scrollYProgress, "change", (v) => setActive(Math.min(n - 1, Math.floor(v * n))));

  const jump = (i: number) => {
    const el = ref.current;
    if (!el) return;
    const top = el.getBoundingClientRect().top + window.scrollY;
    const target = top + ((el.offsetHeight - window.innerHeight) * (i + 0.55)) / n;
    if (lenis) lenis.scrollTo(target, { duration: 1.1 });
    else window.scrollTo({ top: target, behavior: "smooth" });
  };

  return (
    <div ref={ref} className="relative" style={{ height: `${n * 105}vh` }}>
      <div className="sticky top-[4.5rem] mx-auto grid h-[calc(100dvh-4.5rem)] max-w-page grid-cols-[0.82fr_1.18fr] items-center gap-14 px-8">
        <ol className="relative flex flex-col gap-7 pl-12">
          <span aria-hidden="true" className="absolute bottom-3 left-[11px] top-3 w-px bg-border-strong" />
          <m.span aria-hidden="true" className="absolute left-[10px] top-3 w-[3px] origin-top rounded-full bg-text" style={{ height: fill }} />
          {steps.map((step, i) => {
            const on = i === active;
            return (
              <li key={step.key} className="relative">
                <span
                  aria-hidden="true"
                  className={cn(
                    "absolute -left-12 top-2 grid h-6 w-6 place-items-center rounded-full border border-text transition-colors duration-300",
                    i <= active ? "bg-accent" : "bg-surface",
                  )}
                >
                  <span className={cn("h-1.5 w-1.5 rounded-full bg-text transition-opacity", on ? "opacity-100" : "opacity-0")} />
                </span>
                <button type="button" onClick={() => jump(i)} className="block text-left" aria-current={on ? "step" : undefined}>
                  <span className="label">{String(i + 1).padStart(2, "0")}</span>
                  <span className={cn("block font-serif text-[2.6rem] leading-tight transition-colors duration-300", on ? "text-text" : "text-border-strong")}>
                    {step.title}
                  </span>
                </button>
                <AnimatePresence initial={false}>
                  {on && (
                    <m.p
                      initial={{ height: 0, opacity: 0 }}
                      animate={{ height: "auto", opacity: 1 }}
                      exit={{ height: 0, opacity: 0 }}
                      transition={{ duration: 0.35, ease: [0.22, 1, 0.36, 1] }}
                      className="max-w-sm overflow-hidden text-[1.02rem] leading-relaxed text-text-muted"
                    >
                      <span className="block pt-2">{step.body}</span>
                    </m.p>
                  )}
                </AnimatePresence>
              </li>
            );
          })}
        </ol>
        <AnimatePresence mode="wait">
          <m.div
            key={steps[active].key}
            initial={{ opacity: 0, y: 24, rotate: -0.6 }}
            animate={{ opacity: 1, y: 0, rotate: 0 }}
            exit={{ opacity: 0, y: -18, rotate: 0.4 }}
            transition={{ duration: 0.4, ease: [0.22, 1, 0.36, 1] }}
          >
            <Stage step={steps[active]} index={active} total={n} p={locals[active]} />
          </m.div>
        </AnimatePresence>
      </div>
    </div>
  );
}

/** Phones and reduced motion: one card per step, each demo playing through once when it is seen. */
function StackedStep({ step, index, total }: { step: Step; index: number; total: number }) {
  const ref = useRef<HTMLLIElement>(null);
  const seen = useInView(ref, { once: true, margin: "0px 0px -20% 0px" });
  const reduce = useReducedMotion();
  const p = useMotionValue(0);
  useEffect(() => {
    if (!seen) return;
    if (reduce) {
      p.set(1);
      return;
    }
    const run = animate(p, 1, { duration: 3, ease: "easeInOut" });
    return () => run.stop();
  }, [seen, reduce, p]);

  return (
    <li ref={ref} className="flex flex-col gap-5">
      <div>
        <p className="label">{String(index + 1).padStart(2, "0")}</p>
        <h3 className="font-serif text-4xl">{step.title}</h3>
        <p className="mt-2 max-w-md leading-relaxed text-text-muted">{step.body}</p>
      </div>
      <Stage step={step} index={index} total={total} p={p} />
    </li>
  );
}

export function PipelineScroller({ steps }: { steps: Step[] }) {
  const wide = useMedia("(min-width: 1024px) and (prefers-reduced-motion: no-preference)");
  if (wide) return <Pinned steps={steps} />;
  return (
    <ol className="mx-auto flex max-w-page flex-col gap-16 px-5 pb-24 sm:px-8">
      {steps.map((step, i) => (
        <StackedStep key={step.key} step={step} index={i} total={steps.length} />
      ))}
    </ol>
  );
}
