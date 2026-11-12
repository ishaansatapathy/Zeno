"use client";

import { AnimatePresence, m } from "motion/react";
import { useTranslations } from "next-intl";
import { useId, useState } from "react";

import { cn } from "@/components/ui/cn";

const NETS = [
  { key: "slow", quality: "360p", bars: 1, blur: 2.2 },
  { key: "mobile", quality: "480p", bars: 2, blur: 0.9 },
  { key: "wifi", quality: "1080p", bars: 3, blur: 0 },
] as const;

/** The lesson on screen: a hand-drawn leaf in a soft scene, sharper as the connection improves. */
function Frame({ blur }: { blur: number }) {
  return (
    <m.svg
      viewBox="0 0 160 90"
      className="h-full w-full"
      aria-hidden="true"
      animate={{ filter: `blur(${blur}px)` }}
      transition={{ duration: 0.6 }}
    >
      <defs>
        <linearGradient id="sky" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#f6dfae" />
          <stop offset="1" stopColor="#e8c48a" />
        </linearGradient>
      </defs>
      <rect width="160" height="90" fill="url(#sky)" />
      <circle cx="128" cy="22" r="11" fill="#f7b84b" stroke="#17140f" strokeWidth="1.2" />
      <path d="M44 74 C 40 46, 64 24, 98 22 C 100 52, 80 74, 44 74 Z" fill="#7aa36a" stroke="#17140f" strokeWidth="1.4" />
      <path d="M44 74 C 60 58, 74 44, 96 24" fill="none" stroke="#17140f" strokeWidth="1.2" />
      <path d="M58 64 l9 -1 M68 54 l10 -3 M78 44 l9 -4" stroke="#17140f" strokeWidth="1" />
      <rect y="78" width="160" height="12" fill="#cfa86b" />
    </m.svg>
  );
}

/**
 * A student's phone. Pick a connection and the picture adapts: the quality badge changes, the
 * frame gets sharper or softer, and nothing stops to buffer. Playback offers to pick up where
 * it was left.
 */
export function PhoneDemo() {
  const t = useTranslations("site.students");
  const ids = useId();
  const [net, setNet] = useState<(typeof NETS)[number]>(NETS[1]);

  return (
    <div className="flex flex-col items-center gap-8 lg:flex-row lg:items-center lg:justify-center lg:gap-16">
      <fieldset className="order-2 flex flex-col items-center gap-3 lg:order-1 lg:items-start">
        <legend className="label mb-3">{t("network")}</legend>
        <div className="flex gap-2 lg:flex-col">
          {NETS.map((option) => (
            <label key={option.key} className="cursor-pointer">
              <input
                type="radio"
                name={`${ids}-net`}
                checked={net.key === option.key}
                onChange={() => setNet(option)}
                className="peer sr-only"
              />
              <span className="flex items-center gap-3 rounded-full border border-text bg-surface px-4 py-2 text-sm transition-colors peer-checked:bg-text peer-checked:text-bg peer-focus-visible:outline peer-focus-visible:outline-2 peer-focus-visible:outline-focus">
                <span aria-hidden="true" className="flex h-3.5 items-end gap-[2px]">
                  {[1, 2, 3].map((b) => (
                    <span key={b} className={cn("w-[3px] rounded-sm bg-current", b > option.bars && "opacity-25")} style={{ height: `${b * 33}%` }} />
                  ))}
                </span>
                {t(`nets.${option.key}`)}
              </span>
            </label>
          ))}
        </div>
      </fieldset>

      <div className="order-1 w-[17.5rem] rounded-[2.6rem] border-[1.5px] border-text bg-text p-2.5 shadow-[10px_10px_0_0_rgb(var(--accent))] lg:order-2">
        <div className="overflow-hidden rounded-[2.1rem] bg-bg">
          <div className="flex justify-center pt-2.5">
            <span className="h-5 w-20 rounded-full bg-text" />
          </div>
          <div className="px-4 pb-2 pt-3">
            <p className="label">{t("course")}</p>
          </div>
          <div className="relative mx-3 aspect-video overflow-hidden rounded-xl border border-text">
            <Frame blur={net.blur} />
            <AnimatePresence mode="popLayout">
              <m.span
                key={net.quality}
                initial={{ opacity: 0, y: -6 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: 6 }}
                className="absolute right-2 top-2 rounded-full border border-text bg-surface px-2 py-0.5 font-mono text-[0.62rem]"
              >
                {t("quality", { quality: net.quality })}
              </m.span>
            </AnimatePresence>
            <span className="absolute inset-x-2 bottom-2 h-1 overflow-hidden rounded-full bg-text/25">
              <m.span className="block h-full rounded-full bg-accent" initial={{ width: "38%" }} animate={{ width: ["38%", "46%"] }} transition={{ duration: 8, repeat: Infinity, ease: "linear" }} />
            </span>
          </div>
          <div className="px-4 py-3">
            <p className="font-serif text-xl leading-tight">{t("lesson")}</p>
            <p className="mt-2 inline-flex items-center gap-2 rounded-full border border-text px-3 py-1 text-xs">
              <svg viewBox="0 0 12 12" aria-hidden="true" className="h-3 w-3">
                <path d="M3 2 10 6 3 10Z" fill="currentColor" />
              </svg>
              {t("resume", { time: "12:40" })}
            </p>
          </div>
          <ul className="flex flex-col gap-2 border-t border-border px-4 py-3 text-xs text-text-muted" aria-hidden="true">
            {[3, 4, 5].map((n) => (
              <li key={n} className={cn("flex items-center gap-2", n === 4 && "font-medium text-text")}>
                <span className={cn("grid h-4 w-4 place-items-center rounded-full border border-text", n < 4 && "bg-success", n === 4 && "bg-accent")} />
                <span className="h-1.5 flex-1 rounded-full bg-border" />
                <span className="font-mono">{String(n).padStart(2, "0")}</span>
              </li>
            ))}
          </ul>
        </div>
      </div>
    </div>
  );
}
