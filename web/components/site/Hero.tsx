"use client";

import { m, useMotionValueEvent, useReducedMotion, useScroll, useTransform } from "motion/react";
import { useEffect, useRef, useState } from "react";

import { hero, links } from "@/content/site";

import { Button, Container } from "./frame";
import { useMedia } from "./useMedia";

/** A lesson's bitrate across its length: smooth, uneven, the same on every visit. */
function profile(i: number, n: number): number {
  const x = i / n;
  const v = 0.55 + 0.22 * Math.sin(x * 9.3 + 0.6) + 0.14 * Math.sin(x * 23.1 + 2.1) + 0.09 * Math.sin(x * 51.7);
  return Math.min(1, Math.max(0.18, v));
}

const LENGTH_S = 12 * 60 + 48;
const clock = (s: number) => `${String(Math.floor(s / 60)).padStart(2, "0")}:${String(Math.floor(s % 60)).padStart(2, "0")}`;

/**
 * One lesson, drawn as its segments: a bar for each few seconds, as tall as that stretch's
 * bitrate. A thin frost line sweeps across, sealing every segment it passes; then it starts over.
 * Point at a segment to read which one it is and whether it is locked yet.
 */
function Rail({ count }: { count: number }) {
  const reduce = useReducedMotion();
  const [sealed, setSealed] = useState(count);
  const [hover, setHover] = useState<number | null>(null);

  useEffect(() => {
    if (reduce) {
      setSealed(count);
      return;
    }
    setSealed(0);
    const t = window.setInterval(() => setSealed((s) => (s >= count + 26 ? 0 : s + 1)), 70);
    return () => window.clearInterval(t);
  }, [count, reduce]);

  const done = Math.min(sealed, count);
  const status =
    hover !== null
      ? `Segment ${String(hover + 1).padStart(2, "0")} · ${clock((hover / count) * LENGTH_S)} · ${hover < done ? "locked" : "waiting"}`
      : done < count
        ? `Encrypting · ${String(done).padStart(2, "0")} / ${count}`
        : `Locked · ${count} / ${count}`;

  return (
    <div aria-hidden="true">
      <div className="flex items-center justify-between gap-4 border-b border-line px-4 py-3 sm:px-5">
        <span className="font-mono text-[0.72rem] text-paper/80">
          lesson-07.mp4 <span className="text-mute">· 1.24 GB</span>
        </span>
        <span className="label flex items-center gap-2 normal-case tracking-[0.04em]">
          <span className={`size-1.5 rounded-full bg-frost ${done < count && hover === null ? "animate-blink" : ""}`} />
          {status}
        </span>
      </div>
      <div
        className="relative flex h-[clamp(4rem,11svh,6.5rem)] items-end gap-[3px] px-4 pt-5 sm:px-5"
        onPointerMove={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          setHover(Math.min(count - 1, Math.max(0, Math.floor(((e.clientX - r.left) / r.width) * count))));
        }}
        onPointerLeave={() => setHover(null)}
      >
        {Array.from({ length: count }, (_, i) => (
          <span
            key={i}
            style={{ height: `${(profile(i, count) * 100).toFixed(2)}%` }}
            className={`flex-1 rounded-[1px] transition-colors duration-300 ${
              hover === i ? "bg-paper" : i < done ? "bg-frost/75" : i === done ? "bg-paper/70" : "bg-line-hi"
            }`}
          />
        ))}
        {done < count ? (
          <span
            className="pointer-events-none absolute bottom-0 top-3 w-px bg-frost shadow-[0_0_14px_2px_rgba(180,194,214,0.55)] transition-[left] duration-75 ease-linear"
            style={{ left: `calc(1rem + (100% - 2rem) * ${done / count})` }}
          />
        ) : null}
      </div>
      <div className="flex justify-between px-4 pb-3 pt-2.5 font-mono text-[0.65rem] text-mute sm:px-5">
        {[0, 0.25, 0.5, 0.75, 1].map((f) => (
          <span key={f}>{clock(f * LENGTH_S)}</span>
        ))}
      </div>
      <dl className="hidden grid-cols-4 border-t border-line sm:grid [@media(max-height:820px)]:!hidden">
        {[
          ["Qualities", "5 · 144p–720p"],
          ["Cipher", "AES-128"],
          ["Screens", "2 at a time"],
          ["Ledger", "balanced"],
        ].map(([k, v], i) => (
          <div key={k} className={`px-5 py-3 ${i ? "border-l border-line" : ""}`}>
            <dt className="label">{k}</dt>
            <dd className="mt-1 text-[0.85rem] text-paper/90">{v}</dd>
          </div>
        ))}
      </dl>
    </div>
  );
}

/**
 * The opening. A plain statement, and under it one lesson being sealed segment by segment. The
 * hero holds still while you scroll: the words lift away, the lesson's panel lights up, and then
 * it opens out to fill the screen, becoming the light section that follows.
 */
export function Hero() {
  const ref = useRef<HTMLElement>(null);
  const stage = useRef<HTMLDivElement>(null);
  const rail = useRef<HTMLDivElement>(null);
  const insets = useRef({ t: 0, r: 0, b: 0, l: 0 });
  const wide = useMedia("(min-width: 640px)");

  const { scrollYProgress } = useScroll({ target: ref, offset: ["start start", "end end"] });
  const wordsOpacity = useTransform(scrollYProgress, [0.03, 0.26], [1, 0]);
  const wordsY = useTransform(scrollYProgress, [0, 0.26], [0, -50]);
  const light = useTransform(scrollYProgress, [0.24, 0.36], [0, 1]);
  const open = useTransform(scrollYProgress, [0.34, 0.86], [0, 1]);
  const clip = useTransform(open, (e) => {
    const k = Math.pow(1 - e, 3);
    const i = insets.current;
    return `inset(${i.t * k}px ${i.r * k}px ${i.b * k}px ${i.l * k}px round ${12 * k}px)`;
  });

  function measure() {
    const s = stage.current?.getBoundingClientRect();
    const r = rail.current?.getBoundingClientRect();
    if (!s || !r) return;
    insets.current = { t: r.top - s.top, r: s.right - r.right, b: s.bottom - r.bottom, l: r.left - s.left };
  }
  useEffect(() => {
    measure();
    window.addEventListener("resize", measure);
    return () => window.removeEventListener("resize", measure);
  }, []);
  // The panel is still finishing its entrance at first paint; measure again until it starts to open.
  useMotionValueEvent(scrollYProgress, "change", (v) => {
    if (v < 0.34) measure();
  });

  return (
    <section ref={ref} aria-labelledby="hero-title" className="relative h-[180svh]">
      <div ref={stage} className="sticky top-0 flex h-[100svh] flex-col overflow-hidden">
        <div aria-hidden="true" className="pointer-events-none absolute inset-0 bg-[radial-gradient(ellipse_60%_45%_at_50%_0%,rgba(180,194,214,0.07),transparent_70%)]" />

        <m.div style={{ opacity: wordsOpacity, y: wordsY }} className="relative z-10 pt-[clamp(5.5rem,13svh,9.5rem)] text-center">
          <Container>
            <p className="rise inline-flex items-center gap-2 rounded-full border border-line bg-panel/60 px-3 py-1.5 text-[0.78rem] text-mute">
              <span className="size-1.5 rounded-full bg-frost" />
              Video courses and live classes
            </p>
            <h1 id="hero-title" className="display mx-auto mt-6 text-[clamp(2.5rem,min(6.6vw,10svh),6rem)]">
              <span className="rise block" style={{ ["--delay" as string]: "0.06s" }}>
                {hero.title[0]}
              </span>
              <span className="rise block text-mute" style={{ ["--delay" as string]: "0.14s" }}>
                {hero.title[1]}
              </span>
            </h1>
            <p className="rise mx-auto mt-6 max-w-[34rem] text-[0.98rem] leading-relaxed text-mute" style={{ ["--delay" as string]: "0.24s" }}>
              {hero.lines[0]} {hero.lines[1]} <span className="text-paper">{hero.accentLine}</span>
            </p>
            <div className="rise mt-9 flex flex-wrap justify-center gap-3" style={{ ["--delay" as string]: "0.32s" }}>
              <Button href={links.start}>Start teaching</Button>
              <Button href="#bench" tone="line">
                See how it works
              </Button>
            </div>
          </Container>
        </m.div>

        <div className="relative mt-auto pb-[4svh] pt-8">
          <Container>
            <div ref={rail} className="rise mx-auto max-w-[62rem] overflow-hidden rounded-xl border border-line bg-panel/80 backdrop-blur-sm" style={{ ["--delay" as string]: "0.42s" }}>
              <Rail count={wide ? 64 : 36} />
            </div>
          </Container>
        </div>

        {/* The light that opens out of the panel. */}
        <m.div aria-hidden="true" style={{ opacity: light, clipPath: clip }} className="pointer-events-none absolute inset-0 z-20 bg-bone" />
      </div>
    </section>
  );
}
