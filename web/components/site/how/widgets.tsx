"use client";

import { useMotionValueEvent, type MotionValue } from "motion/react";
import { useState } from "react";

import { cn } from "@/components/ui/cn";

/** A whole number that follows a motion value, re-rendering only when it changes. */
export function useStep(p: MotionValue<number>, map: (v: number) => number): number {
  const [value, setValue] = useState(() => map(p.get()));
  useMotionValueEvent(p, "change", (v) => {
    const next = map(v);
    setValue((prev) => (prev === next ? prev : next));
  });
  return value;
}

const clamp = (v: number) => Math.min(1, Math.max(0, v));

/** Upload: 24 parts filling in, in silver. */
export function UploadWidget({ p }: { p: MotionValue<number> }) {
  const done = useStep(p, (v) => Math.round(clamp(v) * 24));
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-baseline justify-between font-mono text-[0.75rem] text-text-muted">
        <span>lesson-07.mp4</span>
        <span className="tabular-nums text-text">{Math.round((done / 24) * 100)}%</span>
      </div>
      <div className="grid grid-cols-8 gap-1.5" aria-hidden="true">
        {Array.from({ length: 24 }, (_, i) => (
          <span
            key={i}
            className={cn(
              "h-3 rounded-[4px] border transition-colors duration-300",
              i < done ? "border-transparent bg-[linear-gradient(180deg,#f4f4f5,#a1a1aa)]" : "border-white/10 bg-white/[0.02]",
            )}
          />
        ))}
      </div>
    </div>
  );
}

const LADDER = [
  { q: "1080p", w: 100 },
  { q: "720p", w: 58 },
  { q: "480p", w: 30 },
  { q: "360p", w: 17 },
  { q: "240p", w: 9 },
];

/** Transcode: the rendition ladder, rung by rung. */
export function TranscodeWidget({ p }: { p: MotionValue<number> }) {
  const shown = useStep(p, (v) => Math.ceil(clamp(v) * LADDER.length));
  return (
    <div className="flex flex-col gap-2" aria-hidden="true">
      {LADDER.map((rung, i) => (
        <div key={rung.q} className="grid grid-cols-[3rem_1fr] items-center gap-3 font-mono text-[0.72rem] text-text-muted">
          <span>{rung.q}</span>
          <span className="h-2 overflow-hidden rounded-full bg-white/[0.05]">
            <span
              className="block h-full rounded-full bg-[linear-gradient(90deg,#a1a1aa,#f4f4f5)] transition-[width] duration-700 ease-out"
              style={{ width: i < shown ? `${rung.w}%` : "0%" }}
            />
          </span>
        </div>
      ))}
    </div>
  );
}

/** Protect: twelve segments, each locking in turn. */
export function ProtectWidget({ p }: { p: MotionValue<number> }) {
  const locked = useStep(p, (v) => Math.round(clamp(v) * 12));
  return (
    <div className="flex flex-col gap-3">
      <div className="grid grid-cols-6 gap-1.5" aria-hidden="true">
        {Array.from({ length: 12 }, (_, i) => (
          <span
            key={i}
            className={cn(
              "grid aspect-video place-items-center rounded-md border transition-colors duration-300",
              i < locked ? "border-white/25 bg-white/[0.08]" : "border-white/10 bg-white/[0.02]",
            )}
          >
            <svg viewBox="0 0 16 16" className={cn("h-3 w-3 transition-opacity duration-300", i < locked ? "opacity-100" : "opacity-0")}>
              <rect x="3.5" y="7" width="9" height="6.5" rx="1.2" fill="#e4e4e7" />
              <path d="M5.5 7V5.3a2.5 2.5 0 0 1 5 0V7" fill="none" stroke="#e4e4e7" strokeWidth="1.5" />
            </svg>
          </span>
        ))}
      </div>
      <p className="font-mono text-[0.72rem] text-text-muted">
        AES-128 · <span className="tabular-nums text-text">{locked}/12</span>
      </p>
    </div>
  );
}

const STUDENTS = [
  { place: "Lagos", q: "480p" },
  { place: "London", q: "1080p" },
  { place: "São Paulo", q: "720p" },
  { place: "Tokyo", q: "1080p" },
];

/** Deliver: students around the world coming online, each at the quality their connection holds. */
export function DeliverWidget({ p }: { p: MotionValue<number> }) {
  const online = useStep(p, (v) => Math.ceil(clamp(v) * STUDENTS.length));
  return (
    <ul className="flex flex-col gap-1.5 font-mono text-[0.75rem]" aria-hidden="true">
      {STUDENTS.map((s, i) => (
        <li
          key={s.place}
          className={cn(
            "flex items-center justify-between rounded-lg border px-3 py-1.5 transition-all duration-500",
            i < online ? "border-white/15 bg-white/[0.05] text-text" : "translate-x-2 border-white/5 text-text-muted opacity-40",
          )}
        >
          <span className="inline-flex items-center gap-2">
            <span className={cn("h-1.5 w-1.5 rounded-full", i < online ? "bg-success" : "bg-white/20")} />
            {s.place}
          </span>
          <span>{s.q}</span>
        </li>
      ))}
    </ul>
  );
}
