"use client";

import { m, useReducedMotion, useScroll, useSpring, useTransform } from "motion/react";
import { useLayoutEffect, useRef, useState } from "react";

export type HeroNode = { key: string; title: string; meta: string };

/** What the backend actually emits when a lesson sells: the event names are the real ones. */
const EVENTS = [
  { at: "09:43:02", name: "payment.captured", ms: 94 },
  { at: "09:43:02", name: "enrollment.granted", ms: 12 },
  { at: "09:43:02", name: "ledger.sale.posted", ms: 8 },
  { at: "09:41:17", name: "lesson.published", ms: 210 },
];

/** Node positions on the card's canvas (percent), left to right with a little rise and fall. */
const PLACES = [
  { x: 4, y: 40 },
  { x: 28, y: 14 },
  { x: 52, y: 52 },
  { x: 76, y: 22 },
];

type Box = { x: number; y: number; w: number; h: number };

/** Curves from each node's right port to the next node's left port, in real pixels. */
function Edges({ boxes, width, height }: { boxes: Box[]; width: number; height: number }) {
  if (boxes.length < 2 || !width) return null;
  return (
    <svg viewBox={`0 0 ${width} ${height}`} className="absolute inset-0 h-full w-full" aria-hidden="true">
      <defs>
        <linearGradient id="edge" x1="0" x2="1">
          <stop offset="0" stopColor="#52525b" />
          <stop offset="0.5" stopColor="#e4e4e7" />
          <stop offset="1" stopColor="#52525b" />
        </linearGradient>
      </defs>
      {boxes.slice(1).map((to, i) => {
        const from = boxes[i];
        const x1 = from.x + from.w;
        const y1 = from.y + from.h / 2;
        const x2 = to.x;
        const y2 = to.y + to.h / 2;
        const bend = Math.max(40, (x2 - x1) / 2);
        const d = `M${x1} ${y1} C ${x1 + bend} ${y1}, ${x2 - bend} ${y2}, ${x2} ${y2}`;
        const delay = 0.8 + i * 0.35;
        return (
          <g key={i}>
            <m.path
              d={d}
              fill="none"
              stroke="url(#edge)"
              strokeWidth="1.2"
              initial={{ pathLength: 0 }}
              animate={{ pathLength: 1 }}
              transition={{ duration: 0.9, delay, ease: [0.65, 0, 0.35, 1] }}
            />
            <circle r="2.6" fill="#f4f4f5">
              <animateMotion dur="3.2s" begin={`${delay + 1}s`} repeatCount="indefinite" path={d} />
            </circle>
          </g>
        );
      })}
    </svg>
  );
}

/**
 * The hero's product card: a lesson's pipeline drawn as nodes, and the last few events below it.
 * It arrives tilted back in perspective and lies flat as the page scrolls up to it, the way a
 * sheet of glass settles onto a table. Springs smooth the scroll so it never jitters.
 */
export function HeroCard({
  nodes,
  title,
  meta,
  status,
  events,
  zoom,
}: {
  nodes: HeroNode[];
  title: string;
  meta: string;
  status: string;
  events: string;
  zoom: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLDivElement>(null);
  const nodeRefs = useRef<(HTMLDivElement | null)[]>([]);
  const [layout, setLayout] = useState<{ boxes: Box[]; width: number; height: number }>({ boxes: [], width: 0, height: 0 });
  const reduce = useReducedMotion();

  // Measure where the nodes really are (their size is in rem, their place in percent), and again
  // whenever the canvas changes size. offsetLeft/Top ignore the card's 3D tilt, which is what we want.
  useLayoutEffect(() => {
    const el = canvas.current;
    if (!el) return;
    const measure = () => {
      const boxes = nodeRefs.current
        .filter((n): n is HTMLDivElement => !!n)
        .map((n) => ({ x: n.offsetLeft, y: n.offsetTop, w: n.offsetWidth, h: n.offsetHeight }));
      setLayout({ boxes, width: el.clientWidth, height: el.clientHeight });
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const { scrollYProgress } = useScroll({ target: ref, offset: ["start end", "start 18%"] });
  const smooth = useSpring(scrollYProgress, { stiffness: 140, damping: 32, mass: 0.6 });
  const rotateX = useTransform(smooth, [0, 1], [reduce ? 0 : 26, 0]);
  const rotateZ = useTransform(smooth, [0, 1], [reduce ? 0 : -3.5, 0]);
  const scale = useTransform(smooth, [0, 1], [reduce ? 1 : 0.9, 1]);
  const lift = useTransform(smooth, [0, 1], [reduce ? 0 : 40, 0]);

  return (
    <div ref={ref} className="[perspective:1800px]">
      <m.div
        style={{ rotateX, rotateZ, scale, y: lift, transformOrigin: "50% 0%" }}
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        transition={{ duration: 1, delay: 0.5 }}
        className="metal overflow-hidden"
      >
        <div className="flex flex-wrap items-center gap-x-4 gap-y-1 border-b border-white/[0.07] px-5 py-4 sm:px-7">
          <p className="text-[0.95rem] text-text">{title}</p>
          <span className="hidden h-4 w-px bg-white/10 sm:block" />
          <p className="font-mono text-[0.8rem] text-text-muted">{meta}</p>
          <p className="ml-auto inline-flex items-center gap-2 font-mono text-[0.78rem] text-success">
            <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-success" />
            {status}
          </p>
        </div>

        <div className="relative grid grid-cols-[auto_1fr]">
          <div className="hidden flex-col gap-2 border-r border-white/[0.06] p-3 sm:flex" aria-hidden="true">
            {["M4 2l7 7-3 1-1 3z", "M8 3v10M3 8h10", "M3 11c2-5 6-6 10-5M10 3l3 3-3 3"].map((d) => (
              <span key={d} className="grid h-8 w-8 place-items-center rounded-lg border border-white/10 bg-white/[0.03]">
                <svg viewBox="0 0 16 16" className="h-3.5 w-3.5 text-text-muted">
                  <path d={d} fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" />
                </svg>
              </span>
            ))}
          </div>

          <div className="relative min-h-[17rem] bg-[radial-gradient(rgb(255_255_255/0.06)_1px,transparent_1.2px)] [background-size:20px_20px] sm:min-h-[19rem]">
            <span className="absolute right-4 top-4 rounded-md border border-white/10 px-2 py-0.5 font-mono text-[0.7rem] text-text-muted">{zoom}</span>
            {/* Desktop: nodes on a canvas, joined by curves with a dot travelling each one. */}
            <div ref={canvas} className="absolute inset-0 hidden sm:block">
              <Edges {...layout} />
              {nodes.map((node, i) => (
                <m.div
                  key={node.key}
                  ref={(el) => {
                    nodeRefs.current[i] = el;
                  }}
                  className="absolute w-[19%] min-w-[9.5rem] rounded-xl border border-white/10 bg-[#121214]/95 px-4 py-3 shadow-[0_12px_30px_-12px_rgb(0_0_0/0.9)]"
                  style={{ left: `${PLACES[i].x}%`, top: `${PLACES[i].y}%` }}
                  initial={{ opacity: 0, y: 10 }}
                  animate={{ opacity: 1, y: 0 }}
                  transition={{ duration: 0.6, delay: 0.7 + i * 0.15, ease: [0.22, 1, 0.36, 1] }}
                >
                  <span className="absolute -left-[3px] top-1/2 h-1.5 w-1.5 -translate-y-1/2 rounded-full border border-white/40 bg-[#050505]" />
                  <span className="absolute -right-[3px] top-1/2 h-1.5 w-1.5 -translate-y-1/2 rounded-full border border-white/40 bg-[#050505]" />
                  <p className="text-[0.92rem] text-text">{node.title}</p>
                  <p className="mt-0.5 truncate font-mono text-[0.72rem] text-text-muted">{node.meta}</p>
                </m.div>
              ))}
            </div>
            {/* Phones: the same four steps as a short list. */}
            <ol className="flex flex-col gap-2 p-4 sm:hidden">
              {nodes.map((node, i) => (
                <li key={node.key} className="flex items-center gap-3 rounded-xl border border-white/10 bg-[#121214] px-4 py-3">
                  <span className="font-mono text-[0.7rem] text-text-muted">{String(i + 1).padStart(2, "0")}</span>
                  <span className="text-[0.92rem]">{node.title}</span>
                  <span className="ml-auto truncate font-mono text-[0.7rem] text-text-muted">{node.meta}</span>
                </li>
              ))}
            </ol>
          </div>
        </div>

        <div className="border-t border-white/[0.07] px-5 py-4 sm:px-7">
          <p className="mb-2 font-mono text-[0.72rem] text-text-muted">{events}</p>
          <ul className="flex flex-col gap-1.5 font-mono text-[0.78rem]">
            {EVENTS.map((event, i) => (
              <m.li
                key={event.name}
                className="grid grid-cols-[auto_1fr_auto] items-center gap-4"
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                transition={{ delay: 1.4 + i * 0.12 }}
              >
                <span className="text-text-muted">
                  <span className={i === 0 ? "text-success" : "text-text-muted"}>●</span> {event.at}
                </span>
                <span className="truncate text-text">{event.name}</span>
                <span className="text-text-muted tabular-nums">{event.ms} ms</span>
              </m.li>
            ))}
          </ul>
        </div>
      </m.div>
    </div>
  );
}
