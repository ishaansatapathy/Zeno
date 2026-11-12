"use client";

import { m, useMotionValueEvent, useTransform, type MotionValue } from "motion/react";
import { useTranslations } from "next-intl";
import { useState } from "react";

import { cn } from "@/components/ui/cn";
import { WORLD, WORLD_DOTS, project } from "@/lib/site/world-dots";

/** A whole number that follows a motion value, re-rendering only when the number changes. */
function useStep(p: MotionValue<number>, map: (v: number) => number): number {
  const [value, setValue] = useState(() => map(p.get()));
  useMotionValueEvent(p, "change", (v) => {
    const next = map(v);
    setValue((prev) => (prev === next ? prev : next));
  });
  return value;
}

const clamp = (v: number) => Math.min(1, Math.max(0, v));

// ---- 01 upload ------------------------------------------------------------------------------------

const PARTS = 24;
/** Parts sent for a given progress: a steady climb, a stall while the connection is down, then the rest. */
function partsSent(p: number): number {
  if (p < 0.42) return Math.round((p / 0.42) * 11);
  if (p < 0.6) return 11;
  return Math.round(11 + ((p - 0.6) / 0.4) * (PARTS - 11));
}

export function UploadDemo({ p }: { p: MotionValue<number> }) {
  const t = useTranslations("site.pipeline.upload");
  const sent = useStep(p, (v) => Math.min(PARTS, partsSent(clamp(v))));
  const phase = useStep(p, (v) => (v >= 0.42 && v < 0.6 ? 1 : v >= 0.6 && v < 0.82 ? 2 : 0));
  const percent = Math.round((sent / PARTS) * 100);

  return (
    <div className="flex h-full flex-col justify-center gap-6">
      <div className="flex items-center gap-4 rounded-control border border-text bg-bg p-4">
        <svg viewBox="0 0 32 40" aria-hidden="true" className="h-10 w-8 shrink-0 text-text">
          <path d="M3 1h18l8 8v30H3z" fill="rgb(var(--surface))" stroke="currentColor" strokeWidth="1.6" strokeLinejoin="round" />
          <path d="M21 1v8h8" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinejoin="round" />
          <path d="M12 18v10l8-5z" fill="currentColor" />
        </svg>
        <div className="min-w-0 flex-1">
          <p className="truncate font-mono text-sm">{t("file")}</p>
          <p className="label mt-1 normal-case tracking-normal">
            {t("size")} · {t("parts", { done: sent, total: PARTS })}
          </p>
        </div>
        <p className="font-serif text-4xl tabular-nums">{percent}%</p>
      </div>
      <div className="grid grid-cols-8 gap-2" aria-hidden="true">
        {Array.from({ length: PARTS }, (_, i) => (
          <m.span
            key={i}
            className="aspect-[5/3] rounded-[6px] border border-text"
            animate={{
              backgroundColor: i < sent ? "rgb(var(--accent))" : "rgb(var(--surface))",
              scale: i === sent - 1 ? [1, 1.12, 1] : 1,
            }}
            transition={{ duration: 0.25 }}
          />
        ))}
      </div>
      <div className="h-7">
        {phase === 1 && (
          <m.p initial={{ opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }} className="label inline-flex items-center gap-2 text-warning">
            <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-warning" /> {t("dropped")}
          </m.p>
        )}
        {phase === 2 && (
          <m.p initial={{ opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }} className="label inline-flex items-center gap-2 text-success">
            <span className="h-1.5 w-1.5 rounded-full bg-success" /> {t("resumed")}
          </m.p>
        )}
      </div>
    </div>
  );
}

// ---- 02 transcode ------------------------------------------------------------------------------

/** A typical rendition ladder: height and bitrate. */
const LADDER = [
  { q: "1080p", mbps: 5.0 },
  { q: "720p", mbps: 2.8 },
  { q: "480p", mbps: 1.4 },
  { q: "360p", mbps: 0.8 },
  { q: "240p", mbps: 0.4 },
];

function Rung({ p, index, q, mbps }: { p: MotionValue<number>; index: number; q: string; mbps: number }) {
  const start = index / LADDER.length;
  const end = (index + 1) / LADDER.length;
  const width = useTransform(p, [start, end], ["0%", `${(mbps / 5) * 100}%`], { clamp: true });
  const done = useStep(p, (v) => (v >= end ? 1 : 0));
  return (
    <div className="grid grid-cols-[3.4rem_1fr_auto] items-center gap-3 sm:grid-cols-[4.2rem_1fr_5rem]">
      <span className="font-mono text-sm">{q}</span>
      <span className="h-6 overflow-hidden rounded-full border border-text bg-bg">
        <m.span className="block h-full rounded-full bg-accent" style={{ width }} />
      </span>
      <span className={cn("flex items-center justify-end gap-1.5 whitespace-nowrap font-mono text-[0.7rem] tabular-nums sm:text-xs", done ? "text-text" : "text-text-muted")}>
        {mbps.toFixed(1)} Mb/s
        <svg viewBox="0 0 12 12" aria-hidden="true" className={cn("h-3 w-3 transition-opacity", done ? "opacity-100" : "opacity-0")}>
          <path d="M2 6.5 5 9.5 10 3" fill="none" stroke="rgb(var(--success))" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </span>
    </div>
  );
}

export function TranscodeDemo({ p }: { p: MotionValue<number> }) {
  const t = useTranslations("site.pipeline.transcode");
  return (
    <div className="flex h-full flex-col justify-center gap-5">
      <p className="label">{t("title")}</p>
      <div className="flex flex-col gap-3.5">
        {LADDER.map((rung, i) => (
          <Rung key={rung.q} p={p} index={i} {...rung} />
        ))}
      </div>
    </div>
  );
}

// ---- 03 protect ----------------------------------------------------------------------------------

const SEGMENTS = 12;

export function ProtectDemo({ p }: { p: MotionValue<number> }) {
  const t = useTranslations("site.pipeline.protect");
  const locked = useStep(p, (v) => Math.min(SEGMENTS, Math.floor(clamp(v) * (SEGMENTS + 1))));
  return (
    <div className="flex h-full flex-col justify-center gap-6">
      <div className="flex items-center justify-between">
        <p className="label">{t("title")}</p>
        <p className="label tabular-nums">
          {t("locked")} {locked}/{SEGMENTS}
        </p>
      </div>
      <div className="grid grid-cols-6 gap-2" aria-hidden="true">
        {Array.from({ length: SEGMENTS }, (_, i) => {
          const on = i < locked;
          return (
            <div key={i} className="relative aspect-video overflow-hidden rounded-[8px] border border-text">
              <div
                className="absolute inset-0"
                style={{
                  background: `linear-gradient(${120 + i * 17}deg, #e9d8b8, #f6eedd 45%, #d8c39b)`,
                }}
              />
              <m.div
                className="absolute inset-0 grid place-items-center bg-[repeating-linear-gradient(135deg,rgb(var(--text))_0_1.5px,transparent_1.5px_6px)]"
                initial={false}
                animate={{ opacity: on ? 1 : 0 }}
                transition={{ duration: 0.3 }}
              >
                <span className="grid h-6 w-6 place-items-center rounded-full border border-text bg-accent">
                  <svg viewBox="0 0 16 16" className="h-3.5 w-3.5">
                    <rect x="3.5" y="7" width="9" height="6.5" rx="1.2" fill="rgb(var(--text))" />
                    <path d="M5.5 7V5.3a2.5 2.5 0 0 1 5 0V7" fill="none" stroke="rgb(var(--text))" strokeWidth="1.5" />
                  </svg>
                </span>
              </m.div>
            </div>
          );
        })}
      </div>
      <div className="flex flex-wrap gap-2">
        <span className="inline-flex items-center gap-2 rounded-full border border-text bg-bg px-3 py-1.5 text-sm">
          <svg viewBox="0 0 16 16" aria-hidden="true" className="h-4 w-4">
            <circle cx="5" cy="8" r="3" fill="none" stroke="currentColor" strokeWidth="1.5" />
            <path d="M8 8h6.5M12 8v2.5M14.5 8v2" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
          </svg>
          {t("key")}
        </span>
        <span className="inline-flex items-center gap-2 rounded-full border border-text bg-bg px-3 py-1.5 text-sm">
          {[0, 1, 2].map((d) => (
            <span key={d} aria-hidden="true" className="h-3.5 w-2.5 rounded-[3px] border border-text bg-accent" />
          ))}
          {t("devices")}
        </span>
      </div>
    </div>
  );
}

// ---- 04 deliver -----------------------------------------------------------------------------------

/** Students around the world, in the order the line reaches them. Places, not borders. */
const CITIES = [
  { name: "New York", lon: -74, lat: 40.7, q: "1080p" },
  { name: "Mexico City", lon: -99.1, lat: 19.4, q: "720p" },
  { name: "São Paulo", lon: -46.6, lat: -23.5, q: "720p" },
  { name: "Lagos", lon: 3.4, lat: 6.5, q: "480p" },
  { name: "London", lon: -0.1, lat: 51.5, q: "1080p" },
  { name: "Nairobi", lon: 36.8, lat: -1.3, q: "480p" },
  { name: "Mumbai", lon: 72.9, lat: 19.1, q: "720p" },
  { name: "Jakarta", lon: 106.8, lat: -6.2, q: "480p" },
  { name: "Tokyo", lon: 139.7, lat: 35.7, q: "1080p" },
  { name: "Sydney", lon: 151.2, lat: -33.9, q: "1080p" },
].map((c) => ({ ...c, ...project(c.lon, c.lat) }));

/** An arc between two places, bowed upward like a flight path. */
function arc(a: { x: number; y: number }, b: { x: number; y: number }): string {
  const mx = (a.x + b.x) / 2;
  const my = (a.y + b.y) / 2 - Math.hypot(b.x - a.x, b.y - a.y) * 0.32;
  return `M${a.x} ${a.y} Q${mx} ${my} ${b.x} ${b.y}`;
}

function Hop({ p, index }: { p: MotionValue<number>; index: number }) {
  const n = CITIES.length;
  const draw = useTransform(p, [index / n, (index + 1) / n], [0, 1], { clamp: true });
  return (
    <m.path
      d={arc(CITIES[index - 1], CITIES[index])}
      fill="none"
      stroke="rgb(var(--text))"
      strokeWidth="0.45"
      strokeLinecap="round"
      style={{ pathLength: draw }}
    />
  );
}

export function DeliverDemo({ p }: { p: MotionValue<number> }) {
  const t = useTranslations("site.pipeline.deliver");
  const reached = useStep(p, (v) => Math.min(CITIES.length, Math.floor(clamp(v) * CITIES.length + 0.999)));
  const latest = CITIES[Math.max(0, reached - 1)];
  return (
    <div className="flex h-full flex-col justify-center gap-4">
      <div className="flex items-center justify-between">
        <p className="label">{t("title")}</p>
        <p className="label tabular-nums">
          {reached}/{CITIES.length}
        </p>
      </div>
      <svg viewBox={`-2 -4 ${WORLD.cols + 4} ${WORLD.rows + 6}`} className="w-full" aria-hidden="true">
        <path d={WORLD_DOTS} stroke="rgb(var(--border-strong))" strokeOpacity="0.55" strokeWidth="0.62" strokeLinecap="round" />
        {CITIES.slice(1).map((_, i) => (
          <Hop key={i} p={p} index={i + 1} />
        ))}
        {CITIES.map((c, i) => {
          const on = i < reached;
          return (
            <g key={c.name} transform={`translate(${c.x} ${c.y})`}>
              {on && i === reached - 1 && (
                <m.circle
                  r="1"
                  fill="none"
                  stroke="rgb(var(--accent))"
                  strokeWidth="0.5"
                  initial={{ scale: 1, opacity: 1 }}
                  animate={{ scale: 4.5, opacity: 0 }}
                  transition={{ duration: 1.4, repeat: Infinity }}
                />
              )}
              <circle r={on ? 1.35 : 0.9} fill={on ? "rgb(var(--accent))" : "rgb(var(--surface))"} stroke="rgb(var(--text))" strokeWidth="0.4" />
            </g>
          );
        })}
      </svg>
      <p className="label h-5 tabular-nums">
        {reached > 0 && (
          <m.span key={latest.name} initial={{ opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }} className="inline-flex items-center gap-2 text-text">
            <span className="h-1.5 w-1.5 rounded-full bg-success" />
            {latest.name} · <span className="normal-case">{latest.q}</span>
          </m.span>
        )}
      </p>
    </div>
  );
}
