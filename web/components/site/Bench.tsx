"use client";

import { m, useInView, useMotionValueEvent, useReducedMotion, useScroll } from "motion/react";
import { useLenis } from "lenis/react";
import { useEffect, useRef, useState, type KeyboardEvent } from "react";

import { bench, type Step } from "@/content/site";

import { Container, SectionHead } from "./frame";
import { useMedia } from "./useMedia";

const RUN_MS = 2600;
const AUTO_MS = 6500;
/** Of each step's stretch of scroll, the share spent running; the rest holds the result. */
const RUN_SHARE = 0.7;
const NAV = 56;

const clamp = (v: number) => Math.min(1, Math.max(0, v));

/** Time-driven run, for when the section is not pinned: 0..1 over RUN_MS, restarting per step. */
function useTimedRun(id: string, go: boolean, enabled: boolean) {
  const reduce = useReducedMotion();
  const [p, setP] = useState(0);
  useEffect(() => {
    if (!enabled) return;
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
  }, [id, go, enabled, reduce]);
  return p;
}

/** One step, at point `p` of its run: the line typed, the pipeline lit stage by stage, the result. */
function Run({ step, p }: { step: Step; p: number }) {
  const typed = Math.round(step.log.length * clamp(p / 0.22));
  const done = p >= 0.86;
  const stageAt = (i: number) => 0.24 + (i * 0.55) / step.stages.length;
  const runTime = (2.4 * clamp(p / 0.86)).toFixed(2);
  const segments = 32;

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center justify-between border-b border-line px-5 py-3 sm:px-6 [@media(max-height:820px)]:py-2.5">
        <p className="label flex items-center gap-2 normal-case tracking-[0.04em]">
          <span className={`size-1.5 rounded-full ${done ? "bg-frost" : "animate-blink bg-paper"}`} />
          <span className={done ? "text-frost" : "text-paper"}>{done ? "Done" : "Running"}</span>
        </p>
        <p className="font-mono text-[0.72rem] text-mute">
          {runTime}s
        </p>
      </div>

      <p className="overflow-hidden whitespace-nowrap border-b border-line px-5 py-3 font-mono text-[0.74rem] text-paper/85 sm:px-6 [@media(max-height:820px)]:py-2.5">
        <span className="text-mute">›</span> {step.log.slice(0, typed)}
        {typed < step.log.length ? <span className="animate-blink text-frost">▍</span> : null}
      </p>

      <div className="flex flex-wrap items-center gap-x-3 gap-y-2 border-b border-line px-5 py-3 sm:px-6 [@media(max-height:820px)]:py-2.5">
        {step.stages.map((s, i) => {
          const lit = p >= stageAt(i);
          return (
            <span key={s.name} className="flex items-center gap-3">
              {i > 0 ? <span aria-hidden="true" className={`hidden h-px w-6 transition-colors duration-500 sm:block lg:w-9 ${lit ? "bg-frost/60" : "bg-line-hi"}`} /> : null}
              <span className="flex items-center gap-2 font-mono text-[0.7rem]">
                <span className={`size-1.5 rounded-full transition-colors duration-300 ${lit ? "bg-frost" : "bg-line-hi"}`} />
                <span className={lit ? "text-paper" : "text-mute"}>{s.name}</span>
                <span className={`transition-opacity duration-300 ${lit ? "text-mute opacity-100" : "opacity-0"}`}>{s.time}</span>
              </span>
            </span>
          );
        })}
      </div>

      <div className="min-h-0 flex-1 overflow-hidden px-5 pb-5 pt-6 sm:px-6 [@media(max-height:820px)]:pb-4 [@media(max-height:820px)]:pt-4">
        <p className="label">{step.claimLabel}</p>
        <p className={`display mt-3 text-[clamp(2.2rem,min(4.2vw,7svh),3.6rem)] transition-[opacity,filter,transform] duration-700 ${p > 0.3 ? "translate-y-0 opacity-100 blur-0" : "translate-y-1 opacity-0 blur-sm"}`}>
          {step.claim[0]} <span className="text-mute">{step.claim[1]}</span>
        </p>

        {step.id === "lock" ? (
          <div aria-hidden="true" className="mt-5 flex gap-[3px] [@media(max-height:820px)]:mt-3">
            {Array.from({ length: segments }, (_, i) => (
              <span key={i} className={`h-4 flex-1 rounded-[1px] transition-colors duration-300 ${p >= 0.3 + (i / segments) * 0.5 ? "bg-frost/75" : "bg-line-hi"}`} />
            ))}
          </div>
        ) : null}

        <div className="mt-6 grid gap-3 sm:grid-cols-[minmax(0,1fr)_12rem] [@media(max-height:820px)]:mt-4">
          <div className={`rounded-lg border px-4 py-3 transition-colors duration-500 ${done ? "border-frost/30 bg-frost/[0.05]" : "border-line"}`}>
            <p className="label">Status</p>
            <p className={`mt-1.5 text-[0.88rem] font-medium ${done ? "text-frost" : "text-mute"}`}>{done ? step.status : "Working…"}</p>
          </div>
          <div className="rounded-lg border border-line px-4 py-3">
            <p className="label">{step.side.k}</p>
            <p className="mt-1.5 font-mono text-[0.85rem] text-paper">{done ? step.side.v : "—"}</p>
          </div>
        </div>
      </div>

      <dl className="grid grid-cols-2 border-t border-line lg:grid-cols-4">
        {step.facts.map((f, i) => (
          <div key={f.k} className={`px-5 py-3.5 sm:px-6 [@media(max-height:820px)]:py-2.5 ${i % 2 ? "border-l border-line" : ""} ${i > 1 ? "border-t border-line lg:border-t-0" : ""} ${i === 2 ? "lg:border-l" : ""}`}>
            <dt className="label">{f.k}</dt>
            <dd className={`mt-1 text-[0.9rem] ${f.accent ? "text-frost" : "text-paper"}`}>{f.v}</dd>
          </div>
        ))}
      </dl>
    </div>
  );
}

/**
 * One lesson through the four steps. On a wide, tall enough screen the section holds still while
 * you scroll, and the scroll drives it: each step gets its stretch, runs through it, then hands on
 * to the next. Clicking a step scrolls to it. Elsewhere (phones, reduced motion) it is a plain set
 * of tabs that run on their own while on screen until someone picks one.
 */
export function Bench() {
  const steps = bench.steps;
  const wide = useMedia("(min-width: 1024px) and (min-height: 620px)");
  const reduce = useReducedMotion();
  const pinned = wide && !reduce;
  const lenis = useLenis();

  const section = useRef<HTMLElement>(null);
  const box = useRef<HTMLDivElement>(null);
  const tabs = useRef<(HTMLButtonElement | null)[]>([]);

  const { scrollYProgress } = useScroll({ target: section, offset: [`start ${NAV}px`, "end end"] });
  const [scrolled, setScrolled] = useState({ active: 0, p: 0 });
  useMotionValueEvent(scrollYProgress, "change", (v) => {
    if (!pinned) return;
    const f = Math.min(steps.length - 0.0001, Math.max(0, v * steps.length));
    const active = Math.floor(f);
    const p = Math.round(clamp((f - active) / RUN_SHARE) * 100) / 100;
    setScrolled((s) => (s.active === active && s.p === p ? s : { active, p }));
  });

  const [picked, setPicked] = useState(0);
  const [touched, setTouched] = useState(false);
  const inView = useInView(box, { amount: 0.4 });
  const [started, setStarted] = useState(false);
  useEffect(() => {
    if (inView) setStarted(true);
  }, [inView]);
  useEffect(() => {
    if (pinned || !inView || touched || reduce) return;
    const t = window.setTimeout(() => setPicked((a) => (a + 1) % steps.length), AUTO_MS);
    return () => window.clearTimeout(t);
  }, [picked, inView, touched, reduce, pinned, steps.length]);

  const active = pinned ? scrolled.active : picked;
  const timed = useTimedRun(steps[active].id, started, !pinned);
  const p = pinned ? scrolled.p : timed;

  function pick(i: number) {
    if (!pinned) {
      setTouched(true);
      setPicked(i);
      return;
    }
    const el = section.current;
    if (!el) return;
    const top = el.getBoundingClientRect().top + window.scrollY - NAV;
    const range = el.offsetHeight - (window.innerHeight - NAV);
    const target = top + range * ((i + 0.02) / steps.length);
    if (lenis) lenis.scrollTo(target, { duration: 1.1 });
    else window.scrollTo({ top: target });
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
    <section
      ref={section}
      id="bench"
      aria-labelledby="bench-title"
      className="relative scroll-mt-14 border-t border-line"
      style={pinned ? { height: `${steps.length * 85 + 100}svh` } : undefined}
    >
      <div className={pinned ? "sticky top-14 flex h-[calc(100svh-3.5rem)] flex-col justify-center py-6" : "py-28"}>
        <Container className={pinned ? "flex min-h-0 flex-col" : ""}>
          <SectionHead id="bench-title" n="03" label={bench.label} title={bench.title} aside={bench.aside} compact={pinned} />

          <div
            ref={box}
            className={`mt-8 grid grid-cols-[minmax(0,1fr)] overflow-hidden rounded-xl border border-line bg-panel/80 lg:mt-10 lg:grid-cols-[17rem_minmax(0,1fr)] ${pinned ? "min-h-0 grid-rows-[minmax(0,1fr)] [@media(max-height:820px)]:lg:mt-6" : ""}`}
          >
            <div className="min-h-0 min-w-0 overflow-hidden border-b border-line lg:border-b-0 lg:border-r">
              <div className="border-b border-line">
                <div className="flex justify-between px-5 py-3">
                  <p className="label">{bench.index}</p>
                  <p className="label">
                    0{active + 1} / 0{steps.length}
                  </p>
                </div>
                {pinned ? <m.span aria-hidden="true" style={{ scaleX: scrollYProgress }} className="block h-px origin-left bg-frost" /> : null}
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
                      className={`relative min-w-[12rem] shrink-0 border-r border-line px-5 py-4 text-left transition-colors duration-300 lg:block lg:w-full lg:min-w-0 lg:border-b lg:border-r-0 ${on ? "bg-raise" : "hover:bg-raise/60"}`}
                    >
                      <span aria-hidden="true" className={`absolute bottom-0 left-0 h-px w-full origin-left bg-frost transition-transform duration-500 lg:top-0 lg:h-full lg:w-px lg:origin-top ${on ? "scale-100" : "scale-0"}`} />
                      <span className={`font-mono text-[0.68rem] ${on ? "text-frost" : "text-mute"}`}>{s.n}</span>
                      <span className={`mt-1.5 block text-[1.02rem] font-medium tracking-[-0.01em] ${on ? "text-paper" : "text-mute"}`}>{s.name}</span>
                      <span className="mt-0.5 block text-[0.8rem] text-mute lg:[@media(max-height:820px)]:hidden">{s.blurb}</span>
                    </button>
                  );
                })}
              </div>
            </div>
            <div role="tabpanel" id="bench-panel" aria-labelledby={`tab-${step.id}`} className={`min-w-0 ${pinned ? "min-h-0 overflow-hidden" : "lg:min-h-[30rem]"}`}>
              <Run key={step.id} step={step} p={p} />
            </div>
          </div>
        </Container>
      </div>
    </section>
  );
}
