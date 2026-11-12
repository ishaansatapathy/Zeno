"use client";

import { AnimatePresence, m, useInView, useReducedMotion } from "motion/react";
import { useTranslations } from "next-intl";
import { useEffect, useRef, useState } from "react";

import { cn } from "@/components/ui/cn";

type Chat = { name: string; text: string; avatar: string };

/** The whiteboard behind the teacher: a leaf, the sun, and what goes in and out. Science, not words. */
function Board() {
  return (
    <svg viewBox="0 0 320 180" className="h-full w-full" aria-hidden="true">
      <g fill="none" stroke="#f3ecdc" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
        <circle cx="52" cy="46" r="15" />
        {Array.from({ length: 8 }, (_, i) => {
          const a = (i * Math.PI) / 4;
          return <path key={i} d={`M${52 + Math.cos(a) * 21} ${46 + Math.sin(a) * 21}L${52 + Math.cos(a) * 28} ${46 + Math.sin(a) * 28}`} />;
        })}
        <path d="M78 62 L128 88" strokeDasharray="5 6" />
        <path d="M122 78 l8 10 -12 2" />
        <path d="M150 140 C 140 90, 180 52, 236 50 C 240 100, 210 140, 150 140 Z" />
        <path d="M150 140 C 178 112, 200 86, 232 54" />
        <path d="M168 126 l14 -2 M184 108 l16 -4 M200 90 l14 -6" />
        <path d="M262 52 l26 -18 M280 32 l8 2 -3 8" />
        <path d="M110 150 l30 -6 M134 140 l6 4 -6 4" />
      </g>
      <g fill="#f3ecdc" fontFamily="var(--font-geist-mono), monospace" fontSize="12">
        <text x="270" y="64">O₂</text>
        <text x="74" y="160">CO₂ + H₂O</text>
      </g>
    </svg>
  );
}

/**
 * A live class in miniature: the board, the LIVE light, a viewer count that keeps climbing and a
 * chat that fills up. Then the class ends and the recording drops into the course as the next
 * lesson. It runs only while it is on screen, and holds its final frame for reduced motion.
 */
export function LiveDemo({ chat, lessonsBefore }: { chat: Chat[]; lessonsBefore: string[] }) {
  const t = useTranslations("site.live");
  const ref = useRef<HTMLDivElement>(null);
  const visible = useInView(ref, { margin: "0px 0px -15% 0px" });
  const reduce = useReducedMotion();
  const [tick, setTick] = useState(0);
  const total = chat.length + 4; // messages, then "ended", then "saved"

  useEffect(() => {
    if (reduce) {
      setTick(total - 1);
      return;
    }
    if (!visible) return;
    const timer = setInterval(() => setTick((v) => (v + 1) % (total + 2)), 1_500);
    return () => clearInterval(timer);
  }, [visible, reduce, total]);

  const shown = Math.min(tick, chat.length);
  const ended = tick > chat.length + 1;
  const saved = tick > chat.length + 2;
  const viewers = 38 + Math.round((Math.min(tick, chat.length) / chat.length) * 88);
  const nextLesson = lessonsBefore.length + 1;

  return (
    <div ref={ref} className="grid gap-4 md:grid-cols-[1.35fr_1fr]">
      <div className="relative overflow-hidden rounded-card border border-text bg-[#2f3a2f] shadow-[8px_8px_0_0_rgb(var(--text))]">
        <div className="aspect-video">
          <Board />
        </div>
        <div className="absolute left-4 top-4 flex items-center gap-2">
          <span className={cn("inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 font-mono text-[0.68rem] uppercase tracking-label", ended ? "bg-[#f3ecdc] text-text" : "bg-danger text-on-danger")}>
            {!ended && <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-on-danger" />}
            {ended ? t("ended") : t("badge")}
          </span>
          {!ended && (
            <span className="rounded-full bg-[#f3ecdc] px-2.5 py-1 font-mono text-[0.68rem] tabular-nums text-text">
              {t("watching", { count: viewers })}
            </span>
          )}
        </div>
        <AnimatePresence>
          {saved && (
            <m.div
              initial={{ opacity: 0, y: 16 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0 }}
              className="absolute inset-x-4 bottom-4 rounded-control border border-text bg-surface p-3"
            >
              <ul className="flex flex-col gap-1.5 text-sm">
                {lessonsBefore.slice(-2).map((lesson, i) => (
                  <li key={lesson} className="flex items-center justify-between text-text-muted">
                    <span>{lesson}</span>
                    <span className="font-mono text-xs">{String(lessonsBefore.length - 1 + i).padStart(2, "0")}</span>
                  </li>
                ))}
                <m.li
                  initial={{ backgroundColor: "rgb(var(--accent))" }}
                  animate={{ backgroundColor: "rgb(var(--accent) / 0.25)" }}
                  transition={{ duration: 1.6 }}
                  className="-mx-1.5 flex items-center justify-between rounded-[6px] px-1.5 py-0.5 font-medium"
                >
                  <span>{t("saved", { n: nextLesson })}</span>
                  <span className="font-mono text-xs">{String(nextLesson).padStart(2, "0")}</span>
                </m.li>
              </ul>
            </m.div>
          )}
        </AnimatePresence>
      </div>

      <div className="flex flex-col rounded-card border border-text bg-surface">
        <p className="label border-b border-text px-4 py-3">{t("chatLabel")}</p>
        <ul className="flex min-h-[15rem] flex-1 flex-col justify-end gap-3 overflow-hidden p-4" aria-live="off">
          <AnimatePresence initial={false}>
            {chat.slice(Math.max(0, shown - 4), shown).map((message) => (
              <m.li
                key={message.name}

                initial={{ opacity: 0, y: 14, scale: 0.98 }}
                animate={{ opacity: 1, y: 0, scale: 1 }}
                exit={{ opacity: 0, y: -10 }}
                transition={{ type: "spring", stiffness: 300, damping: 26 }}
                className="flex items-start gap-2.5"
              >
                <span
                  aria-hidden="true"
                  className="h-8 w-8 shrink-0 overflow-hidden rounded-full border border-text bg-accent/40 [&>svg]:h-full [&>svg]:w-full"
                  dangerouslySetInnerHTML={{ __html: message.avatar }}
                />
                <span className="text-sm leading-snug">
                  <span className="font-medium">{message.name}</span>
                  <span className="block text-text-muted">{message.text}</span>
                </span>
              </m.li>
            ))}
          </AnimatePresence>
        </ul>
      </div>
    </div>
  );
}
