"use client";

import { AnimatePresence, m, useInView, useMotionValueEvent, useReducedMotion, useScroll } from "motion/react";
import { useLenis } from "lenis/react";
import { useEffect, useRef, useState, type CSSProperties, type KeyboardEvent, type ReactNode } from "react";

import { bench, type FlowNode } from "@/content/site";

import { Container, Label } from "./frame";
import { useMedia } from "./useMedia";

const RUN_MS = 2800;
const AUTO_MS = 6000;
/** Of each step's stretch of scroll, the share spent drawing; the rest holds the result. */
const RUN_SHARE = 0.72;
/** The floating nav's height plus its gap from the top. */
const NAV = 64;
const W = 1000;
const H = 620;
const NODE_H = 150;

const clamp = (v: number) => Math.min(1, Math.max(0, v));

const icons: Record<FlowNode["icon"] | "plus", ReactNode> = {
  plus: <path d="M8 3v10M3 8h10" />,
  upload: <path d="M8 11V3M4.5 6.5 8 3l3.5 3.5M3 13h10" />,
  layers: <path d="m8 2.5 5.5 3L8 8.5 2.5 5.5 8 2.5ZM2.5 8.5 8 11.5l5.5-3M2.5 11 8 14l5.5-3" />,
  lock: (
    <>
      <rect x="3.5" y="7" width="9" height="6.5" />
      <path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2" />
    </>
  ),
  card: (
    <>
      <rect x="2" y="4" width="12" height="8.5" />
      <path d="M2 7h12M4.5 10h2.5" />
    </>
  ),
  ledger: (
    <>
      <rect x="2.5" y="2.5" width="11" height="11" />
      <path d="M8 2.5v11M4.5 6h2M9.5 6h2M4.5 9h2M9.5 9h2" />
    </>
  ),
  bank: <path d="M2.5 6 8 3l5.5 3M3.5 6.5v5M6.5 6.5v5M9.5 6.5v5M12.5 6.5v5M2.5 13h11" />,
};

function Icon({ name, className = "size-3.5" }: { name: keyof typeof icons; className?: string }) {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false" className={`fill-none stroke-current ${className}`} strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round">
      {icons[name]}
    </svg>
  );
}

/** 0..1 over one run of a step, driven by time; for when the section is not pinned. */
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

const byId = new Map<string, FlowNode>(bench.nodes.map((n) => [n.id, n]));

/** The link between two nodes: right side to left side as a smooth S, or straight down when stacked. */
function edgeGeometry(fromId: string, toId: string) {
  const a = byId.get(fromId)!;
  const b = byId.get(toId)!;
  if (Math.abs(a.x - b.x) < 10) {
    const x = a.x + a.w / 2;
    const y1 = a.y + NODE_H;
    const y2 = b.y;
    return { d: `M${x},${y1} L${x},${y2}`, mid: [x, (y1 + y2) / 2] };
  }
  const x1 = a.x + a.w;
  const y1 = a.y + NODE_H / 2;
  const x2 = b.x;
  const y2 = b.y + NODE_H / 2;
  const cx = (x1 + x2) / 2;
  // The midpoint of a cubic with both control points at the halfway x is exactly the halfway point.
  return { d: `M${x1},${y1} C${cx},${y1} ${cx},${y2} ${x2},${y2}`, mid: [cx, (y1 + y2) / 2] };
}

/**
 * Where each node and link is at point `p` of step `active`: earlier steps are all drawn; the
 * current step draws its links one after another, each node arriving as its link reaches it.
 */
function layout(active: number, p: number) {
  const edges = bench.edges.map((e) => {
    const siblings = bench.edges.filter((x) => x.step === e.step);
    const k = siblings.indexOf(e);
    const start = 0.08 + k * 0.16;
    const t = e.step < active ? 1 : e.step > active ? 0 : clamp((p - start) / 0.3);
    return { ...e, t, ...edgeGeometry(e.from, e.to) };
  });
  const nodes = bench.nodes.map((n) => {
    let shown: boolean;
    if (n.step < active) shown = true;
    else if (n.step > active) shown = false;
    else {
      const incoming = edges.find((e) => e.to === n.id);
      shown = incoming ? incoming.t >= 0.92 : p >= 0.02;
    }
    return { ...n, shown, current: n.step === active };
  });
  return { edges, nodes };
}

/** One card on the canvas: what kind of thing it is, what happened, and its status. */
function Card({ n, glow, current, className = "", style }: { n: FlowNode; glow: boolean; current: boolean; className?: string; style?: CSSProperties }) {
  return (
    <div
      className={`flex flex-col rounded-xl border bg-raise/95 p-4 ${glow ? "border-silver/70 shadow-[0_0_0_1px_rgba(228,231,235,0.25),0_0_40px_rgba(228,231,235,0.18)]" : current ? "border-silver/35" : "border-line"} ${className}`}
      style={style}
    >
      <p className="flex items-center gap-2 text-[0.74rem] text-mute">
        <Icon name={n.icon} />
        {n.meta}
      </p>
      <p className="mt-2 truncate text-[1rem] font-semibold tracking-[-0.01em] text-paper">{n.title}</p>
      <p className="mt-1 truncate text-[0.82rem] text-mute">{n.line}</p>
      <span className="mt-auto inline-flex w-fit items-center gap-1.5 rounded-md border border-line-hi px-2 py-1 text-[0.72rem] text-paper/85">
        <span className="text-silver">▷</span> {n.chip}
      </span>
    </div>
  );
}

/** On a phone: the same cards in one column, joined top to bottom; ones still to come wait faintly. */
function Stack({ active, p }: { active: number; p: number }) {
  const { edges, nodes } = layout(active, p);
  const done = p >= 0.86;
  return (
    <div aria-hidden="true" className="overflow-hidden rounded-2xl border border-line bg-panel">
      <div className="flex items-center justify-between border-b border-line px-4 py-3">
        <p className="text-[0.88rem] font-medium text-paper">{bench.canvas}</p>
        <p className="font-mono text-[0.7rem] text-mute">{bench.canvasMeta}</p>
      </div>
      <div className="p-4" style={{ backgroundImage: "radial-gradient(#26262a 1px, transparent 1.3px)", backgroundSize: "22px 22px" }}>
        {nodes.map((n, i) => {
          const incoming = edges.find((e) => e.to === n.id);
          return (
            <div key={n.id}>
              {i > 0 ? (
                <div className="relative ml-7 h-9">
                  <span className={`absolute inset-y-0 left-0 w-px transition-colors duration-500 ${incoming && incoming.t > 0.5 ? "bg-silver/60" : "bg-line-hi"}`} />
                  {incoming ? (
                    <span className={`absolute left-0 top-1/2 grid size-5 -translate-x-1/2 -translate-y-1/2 place-items-center rounded-full border bg-ink font-mono text-[0.6rem] transition-colors duration-500 ${incoming.t > 0.5 ? "border-silver/70 text-silver-hi" : "border-line-hi text-mute"}`}>
                      {incoming.badge}
                    </span>
                  ) : null}
                </div>
              ) : null}
              <Card
                n={n}
                glow={n.id === "payout" && n.shown && active === 3 && done}
                current={n.current && n.shown}
                className={`h-[8.5rem] transition-[opacity,border-color] duration-500 ${n.shown ? "opacity-100" : "border-dashed opacity-25"}`}
              />
            </div>
          );
        })}
      </div>
    </div>
  );
}

/** The canvas: a dotted board with a tool rail, cards for what has happened, and the links between. */
function Canvas({ active, p, pinned }: { active: number; p: number; pinned: boolean }) {
  const wrap = useRef<HTMLDivElement>(null);
  const [scale, setScale] = useState(0.9);
  const { edges, nodes } = layout(active, p);
  const done = p >= 0.86;

  useEffect(() => {
    const el = wrap.current;
    if (!el) return;
    const fit = () => {
      const byWidth = el.clientWidth / W;
      const byHeight = pinned ? (window.innerHeight - NAV - 120) / H : Infinity;
      setScale(Math.max(0.62, Math.min(byWidth, byHeight, 1.05)));
    };
    fit();
    const ro = new ResizeObserver(fit);
    ro.observe(el);
    window.addEventListener("resize", fit);
    return () => {
      ro.disconnect();
      window.removeEventListener("resize", fit);
    };
  }, [pinned]);

  // On narrow screens the board scrolls sideways; keep the newest card in view.
  useEffect(() => {
    const el = wrap.current;
    if (!el || el.scrollWidth <= el.clientWidth) return;
    const newest = bench.nodes.filter((n) => n.step === active)[0];
    if (newest) el.scrollTo({ left: Math.max(0, newest.x * scale - 24), behavior: "smooth" });
  }, [active, scale]);

  return (
    <div aria-hidden="true" className="relative overflow-hidden rounded-2xl border border-line bg-panel shadow-[0_50px_140px_-50px_rgba(228,231,235,0.16)]">
      <div className="flex items-center justify-between border-b border-line px-5 py-3.5">
        <p className="text-[0.9rem] font-medium text-paper">{bench.canvas}</p>
        <p className="font-mono text-[0.72rem] text-mute">{bench.canvasMeta}</p>
      </div>
      <div ref={wrap} className="overflow-x-auto overflow-y-hidden">
        <div className="relative mx-auto" style={{ width: W * scale, height: H * scale }}>
          <div
            className="absolute left-0 top-0 origin-top-left"
            style={{
              width: W,
              height: H,
              transform: `scale(${scale})`,
              backgroundImage: "radial-gradient(#26262a 1px, transparent 1.3px)",
              backgroundSize: "22px 22px",
            }}
          >
            {/* Tool rail. */}
            <div className="absolute left-4 top-4 flex flex-col gap-2">
              {(["plus", "upload", "layers", "lock", "card"] as const).map((name, i) => (
                <span
                  key={name}
                  className={`grid size-9 place-items-center rounded-lg border transition-colors duration-300 ${
                    i - 1 === active ? "border-silver/40 bg-raise text-silver-hi" : "border-line bg-ink/70 text-mute"
                  }`}
                >
                  <Icon name={name} className="size-4" />
                </span>
              ))}
            </div>

            {/* Links. */}
            <svg className="absolute inset-0" width={W} height={H} viewBox={`0 0 ${W} ${H}`}>
              {edges.map((e) => (
                <g key={`${e.from}-${e.to}`}>
                  <path d={e.d} fill="none" stroke="#2e2e2e" strokeWidth="1.2" strokeDasharray="3 5" opacity={e.t > 0 ? 0 : 0.7} />
                  <path d={e.d} fill="none" stroke="#c9ced6" strokeWidth="1.4" pathLength={1} strokeDasharray="1" strokeDashoffset={1 - e.t} opacity={0.75} />
                  <g opacity={e.t >= 0.5 ? 1 : 0} style={{ transition: "opacity 300ms" }}>
                    <circle cx={e.mid[0]} cy={e.mid[1]} r="11" fill="#080808" stroke="#9aa1ab" strokeWidth="1" />
                    <text x={e.mid[0]} y={e.mid[1] + 3.5} textAnchor="middle" fontSize="10" fill="#e4e7eb" fontFamily="var(--font-mono)">
                      {e.badge}
                    </text>
                  </g>
                </g>
              ))}
            </svg>

            {/* Cards. */}
            {nodes.map((n) => (
              <Card
                key={n.id}
                n={n}
                glow={n.id === "payout" && n.shown && active === 3 && done}
                current={n.current && n.shown}
                className={`absolute transition-[opacity,transform,border-color,box-shadow] duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] ${n.shown ? "translate-y-0 scale-100 opacity-100" : "translate-y-2 scale-[0.97] opacity-0"}`}
                style={{ left: n.x, top: n.y, width: n.w, height: NODE_H }}
              />
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}

/**
 * One lesson through Zeno, as a flow. On the left: the four steps, what the current one does, and
 * the line it runs. On the right: a canvas where each step adds its cards and draws its links. On
 * a wide, tall enough screen the section holds still and the scroll drives it; elsewhere it steps
 * through on its own until someone picks a step.
 */
export function Flow() {
  const steps = bench.steps;
  const wide = useMedia("(min-width: 1024px) and (min-height: 620px)");
  const narrow = useMedia("(max-width: 639px)");
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
  const inView = useInView(box, { amount: 0.35 });
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
  const step = steps[active];

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
    const by = e.key === "ArrowRight" || e.key === "ArrowDown" ? 1 : e.key === "ArrowLeft" || e.key === "ArrowUp" ? -1 : 0;
    if (!by) return;
    e.preventDefault();
    const next = (active + by + steps.length) % steps.length;
    pick(next);
    tabs.current[next]?.focus();
  }

  return (
    <section ref={section} id="bench" aria-labelledby="bench-title" className="relative scroll-mt-16" style={pinned ? { height: `${steps.length * 90 + 100}svh` } : undefined}>
      <div className={pinned ? "sticky top-16 flex h-[calc(100svh-4rem)] items-center py-5" : "py-28"}>
        <Container>
          <div ref={box} className="grid gap-10 lg:grid-cols-[minmax(0,25rem)_minmax(0,1fr)] lg:items-center lg:gap-14">
            <div>
              <Label>{bench.label}</Label>
              <div role="tablist" aria-label="Steps" onKeyDown={onKey} className="mt-6 flex flex-wrap gap-2">
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
                      aria-controls="flow-panel"
                      tabIndex={on ? 0 : -1}
                      onClick={() => pick(i)}
                      className={`h-9 rounded-full border px-4 text-[0.88rem] font-medium transition-colors duration-300 ${
                        on ? "border-silver bg-silver text-ink" : "border-line-hi text-paper/85 hover:border-mute"
                      }`}
                    >
                      {s.name}
                    </button>
                  );
                })}
              </div>

              <div id="flow-panel" role="tabpanel" aria-labelledby={`tab-${step.id}`}>
                <p className="mt-8 font-mono text-[0.75rem] text-mute">
                  {step.n} / 0{steps.length}
                </p>
                <AnimatePresence mode="wait" initial={false}>
                  <m.div
                    key={step.id}
                    initial={{ opacity: 0, y: 12, filter: "blur(6px)" }}
                    animate={{ opacity: 1, y: 0, filter: "blur(0px)" }}
                    exit={{ opacity: 0, y: -8, filter: "blur(4px)" }}
                    transition={{ duration: 0.45, ease: [0.22, 1, 0.36, 1] }}
                  >
                    <h2 id="bench-title" className="display mt-4 text-[clamp(2.5rem,min(4.4vw,8.5svh),4.4rem)]">
                      <span className="block">{step.claim[0]}</span>
                      <span className="block">
                        <em className="metal">{step.claim[1]}</em>
                      </span>
                    </h2>
                    <p className="mt-5 max-w-[24rem] text-[1rem] leading-relaxed text-mute">{step.desc}</p>
                  </m.div>
                </AnimatePresence>
              </div>

              {/* The step's numbers, and how far through it the scroll has come. */}
              <div className="mt-8 border-t border-line pt-5">
                <dl key={step.id} className="grid grid-cols-3 gap-3">
                  {step.stats.map((st, i) => (
                    <m.div
                      key={st.k}
                      initial={{ opacity: 0, y: 8 }}
                      animate={{ opacity: 1, y: 0 }}
                      transition={{ duration: 0.45, delay: 0.1 + i * 0.07, ease: [0.22, 1, 0.36, 1] }}
                    >
                      <dt className="label">{st.k}</dt>
                      <dd className="display mt-1.5 text-[clamp(1.6rem,2.2vw,2.1rem)]">{st.v}</dd>
                    </m.div>
                  ))}
                </dl>
                <div aria-hidden="true" className="mt-5 flex gap-1.5">
                  {steps.map((s, i) => (
                    <span key={s.id} className="h-px flex-1 overflow-hidden bg-line-hi">
                      <span
                        className="block h-full origin-left bg-silver"
                        style={{ transform: `scaleX(${i < active ? 1 : i > active ? 0 : Math.min(1, p / 0.86)})` }}
                      />
                    </span>
                  ))}
                </div>
              </div>
            </div>

            {narrow ? <Stack active={active} p={p} /> : <Canvas active={active} p={p} pinned={pinned} />}
          </div>
        </Container>
      </div>
    </section>
  );
}
