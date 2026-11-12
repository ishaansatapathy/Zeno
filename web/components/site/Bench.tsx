"use client";

import { useInView, useReducedMotion } from "motion/react";
import { useEffect, useRef, useState, type KeyboardEvent } from "react";

import { bench, type Step } from "@/content/site";

import { Crosses, SectionHead } from "./frame";

const RUN_MS = 2600;
const AUTO_MS = 6500;

/** 0..1 over one run of a step; restarts whenever the step changes. Instantly 1 for reduced motion. */
function useRun(id: string, go: boolean) {
  const reduce = useReducedMotion();
  const [p, setP] = useState(0);
  useEffect(() => {
    if (reduce) {
      setP(1);
      return;
    }
    if (!go) return;
    setP(0);
    let frame = 0;
    const start = performance.now();
    const tick = (now: number) => {
      const v = Math.min(1, (now - start) / RUN_MS);
      setP(v);
      if (v < 1) frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [id, go, reduce]);
  return p;
}

const clamp = (v: number) => Math.min(1, Math.max(0, v));

/** One step, running: the command typed, the pipeline lighting up stage by stage, the result. */
function Run({ step, go }: { step: Step; go: boolean }) {
  const p = useRun(step.id, go);
  const typed = Math.round(step.log.length * clamp(p / 0.22));
  const done = p >= 0.86;
  const stageAt = (i: number) => 0.24 + (i * 0.55) / step.stages.length;
  const runTime = (2.4 * clamp(p / 0.86)).toFixed(2);
  const segments = 24;

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center justify-between border-b border-line px-5 py-3.5 sm:px-7">
        <p className={`label flex items-center gap-2 ${done ? "text-signal" : "text-paper"}`}>
          <span className={`size-1.5 bg-signal ${done ? "" : "animate-blink"}`} />
          {done ? "Done" : "Running"}
        </p>
        <p className="label">
          Run time <span className="text-paper">{runTime}s</span>
        </p>
      </div>

      <p className="overflow-hidden whitespace-nowrap border-b border-line px-5 py-3.5 font-mono text-[0.75rem] text-paper/85 sm:px-7">
        <span className="text-signal">›</span> {step.log.slice(0, typed)}
        {typed < step.log.length ? <span className="animate-blink">▍</span> : null}
      </p>

      <div className="flex flex-wrap items-center gap-x-3 gap-y-2 border-b border-line px-5 py-3.5 sm:px-7">
        <span className="label mr-2">Pipeline</span>
        {step.stages.map((s, i) => {
          const lit = p >= stageAt(i);
          return (
            <span key={s.name} className="flex items-center gap-3">
              {i > 0 ? <span aria-hidden="true" className={`hidden h-px w-6 border-t border-dashed sm:block lg:w-10 ${lit ? "border-signal" : "border-line-hi"}`} /> : null}
              <span className="flex items-center gap-2 font-mono text-[0.7rem] uppercase tracking-[0.08em]">
                <span className={`size-2 transition-colors duration-300 ${lit ? "bg-signal" : "border border-line-hi"}`} />
                <span className={lit ? "text-paper" : "text-mute"}>{s.name}</span>
                <span className={`transition-opacity duration-300 ${lit ? "text-signal opacity-100" : "opacity-0"}`}>{s.time}</span>
              </span>
            </span>
          );
        })}
      </div>

      <div className="flex-1 px-5 pb-6 pt-7 sm:px-7">
        <p className="label">{step.claimLabel}</p>
        <p className={`display mt-4 text-[clamp(2.2rem,4.4vw,3.8rem)] transition-[opacity,filter] duration-700 ${p > 0.3 ? "opacity-100 blur-0" : "opacity-0 blur-sm"}`}>
          {step.claim[0]} <em className="italic text-signal">{step.claim[1]}</em>
        </p>

        {step.id === "lock" ? (
          <div aria-hidden="true" className="mt-6 flex gap-1">
            {Array.from({ length: segments }, (_, i) => (
              <span key={i} className={`h-6 flex-1 transition-colors duration-200 ${p >= 0.3 + (i / segments) * 0.5 ? "bg-signal" : "bg-line"}`} />
            ))}
          </div>
        ) : null}

        <div className="mt-7 grid gap-3 sm:grid-cols-[minmax(0,1fr)_12rem]">
          <div className={`border px-5 py-4 transition-colors duration-500 ${done ? "border-signal/60 bg-signal/[0.07]" : "border-line"}`}>
            <p className="label">Status</p>
            <p className={`caps mt-2 text-[0.8rem] ${done ? "text-signal" : "text-mute"}`}>→ {done ? step.status : "Working"}</p>
          </div>
          <div className="border border-line px-5 py-4">
            <p className="label">{step.side.k}</p>
            <p className="mt-2 font-mono text-[0.85rem] text-paper">{done ? step.side.v : "…"}</p>
          </div>
        </div>
      </div>

      <dl className="grid grid-cols-2 border-t border-line lg:grid-cols-4">
        {step.facts.map((f, i) => (
          <div key={f.k} className={`px-5 py-4 sm:px-7 ${i % 2 ? "border-l border-line" : ""} ${i > 1 ? "border-t border-line lg:border-t-0" : ""} ${i === 2 ? "lg:border-l" : ""} ${f.accent ? "bg-signal/[0.06]" : ""}`}>
            <dt className="label">{f.k}</dt>
            <dd className={`mt-1.5 text-[0.95rem] ${f.accent ? "display italic text-signal" : "text-paper"}`}>{f.v}</dd>
          </div>
        ))}
      </dl>
    </div>
  );
}

/**
 * A working bench: the four steps on the left, one lesson going through the chosen step on the
 * right. It steps through on its own while on screen until someone picks a step themselves.
 */
export function Bench() {
  const steps = bench.steps;
  const [active, setActive] = useState(0);
  const [touched, setTouched] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  const tabs = useRef<(HTMLButtonElement | null)[]>([]);
  const inView = useInView(box, { amount: 0.4 });
  const reduce = useReducedMotion();
  const [started, setStarted] = useState(false);

  useEffect(() => {
    if (inView) setStarted(true);
  }, [inView]);

  useEffect(() => {
    if (!inView || touched || reduce) return;
    const t = window.setTimeout(() => setActive((a) => (a + 1) % steps.length), AUTO_MS);
    return () => window.clearTimeout(t);
  }, [active, inView, touched, reduce, steps.length]);

  function pick(i: number) {
    setTouched(true);
    setActive(i);
  }

  function onKey(e: KeyboardEvent) {
    const by = e.key === "ArrowDown" || e.key === "ArrowRight" ? 1 : e.key === "ArrowUp" || e.key === "ArrowLeft" ? -1 : 0;
    if (!by) return;
    e.preventDefault();
    const next = (active + by + steps.length) % steps.length;
    pick(next);
    tabs.current[next]?.focus();
  }

  const step = steps[active];

  return (
    <section id="bench" aria-labelledby="bench-title" className="mx-auto max-w-page scroll-mt-12 px-4 py-28 sm:px-6 lg:px-10 lg:py-36">
      <SectionHead id="bench-title" label={bench.label} title={bench.title} aside={bench.aside} />

      <div ref={box} className="relative mt-16 grid border border-line bg-ink/60 lg:mt-20 lg:grid-cols-[19rem_minmax(0,1fr)]">
        <Crosses />
        <div className="border-b border-line lg:border-b-0 lg:border-r">
          <div className="flex justify-between border-b border-line px-5 py-3.5">
            <p className="label">{bench.index}</p>
            <p className="label">
              0{active + 1} / 0{steps.length}
            </p>
          </div>
          <div role="tablist" aria-label="Steps" aria-orientation="vertical" onKeyDown={onKey} className="flex overflow-x-auto lg:block">
            {steps.map((s, i) => {
              const on = i === active;
              return (
                <button
                  key={s.id}
                  ref={(el) => {
                    tabs.current[i] = el;
                  }}
                  role="tab"
                  id={`tab-${s.id}`}
                  aria-selected={on}
                  aria-controls="bench-panel"
                  tabIndex={on ? 0 : -1}
                  onClick={() => pick(i)}
                  className={`relative min-w-[13rem] shrink-0 border-r border-line px-5 py-5 text-left transition-colors duration-300 lg:block lg:w-full lg:min-w-0 lg:border-b lg:border-r-0 lg:py-6 ${on ? "bg-signal/[0.07]" : "hover:bg-paper/[0.03]"}`}
                >
                  <span aria-hidden="true" className={`absolute bottom-0 left-0 h-0.5 w-full origin-left bg-signal transition-transform duration-500 lg:top-0 lg:h-full lg:w-0.5 lg:origin-top ${on ? "scale-100" : "scale-0"}`} />
                  <span className={`label block ${on ? "text-signal" : ""}`}>Step · {s.n}</span>
                  <span className="display mt-2 block text-[1.45rem]">{s.name}</span>
                  <span className="mt-1 block text-[0.8rem] text-mute">{s.blurb}</span>
                </button>
              );
            })}
          </div>
        </div>
        <div role="tabpanel" id="bench-panel" aria-labelledby={`tab-${step.id}`} className="min-w-0 lg:min-h-[34rem]">
          <Run key={step.id} step={step} go={started} />
        </div>
      </div>
    </section>
  );
}
