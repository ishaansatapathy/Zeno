"use client";

import { useLenis } from "lenis/react";
import { animate, useInView, useMotionValue, useMotionValueEvent, useReducedMotion, useScroll, useTransform, type MotionValue } from "motion/react";
import { useEffect, useRef, useState, type ComponentType } from "react";

import { cn } from "@/components/ui/cn";

import { useMedia } from "../useMedia";

import { DeliverWidget, ProtectWidget, TranscodeWidget, UploadWidget } from "./widgets";

export type Column = { key: "upload" | "transcode" | "protect" | "deliver"; label: string; title: string; body: string };

const WIDGETS: Record<Column["key"], ComponentType<{ p: MotionValue<number> }>> = {
  upload: UploadWidget,
  transcode: TranscodeWidget,
  protect: ProtectWidget,
  deliver: DeliverWidget,
};

const clamp = (v: number) => Math.min(1, Math.max(0, v));

/** One column: narrow and quiet when closed, wide with its widget when open. */
function Pillar({ column, index, open, p, onOpen }: { column: Column; index: number; open: boolean; p: MotionValue<number>; onOpen: () => void }) {
  const Widget = WIDGETS[column.key];
  return (
    <button
      type="button"
      onClick={onOpen}
      aria-expanded={open}
      className={cn(
        "metal group relative flex h-full min-w-0 basis-0 flex-col overflow-hidden p-6 text-left transition-[flex-grow] duration-700 ease-[cubic-bezier(0.22,1,0.36,1)]",
        open ? "grow-[4]" : "grow hover:bg-white/[0.02]",
      )}
    >
      <span
        aria-hidden="true"
        className={cn(
          "pointer-events-none absolute inset-0 rounded-[inherit] bg-[radial-gradient(120%_70%_at_20%_0%,rgb(255_255_255/0.11),transparent_60%)] transition-opacity duration-700",
          open ? "opacity-100" : "opacity-0",
        )}
      />
      <span className="relative font-mono text-[0.72rem] text-text-muted">{String(index + 1).padStart(3, "0")}</span>
      <span
        className={cn(
          "relative mt-6 flex w-[min(28rem,48vw)] flex-col gap-5 transition-[opacity,transform] duration-500",
          open ? "translate-y-0 opacity-100 delay-200" : "pointer-events-none translate-y-3 opacity-0",
        )}
      >
        <span className="headline silver block max-w-md text-[2.2rem]">{column.title}</span>
        <span className="block max-w-md text-[1rem] leading-relaxed text-text-muted">{column.body}</span>
        <span className="mt-2 block max-w-sm">
          <Widget p={p} />
        </span>
      </span>
      <span className="shout relative mt-auto block text-[2.1rem] text-text/90">{column.label}</span>
    </button>
  );
}

/** Desktop: the row holds still while the page scrolls, and the scroll opens each column in turn. */
function Pinned({ columns }: { columns: Column[] }) {
  const ref = useRef<HTMLDivElement>(null);
  const lenis = useLenis();
  const n = columns.length;
  const { scrollYProgress } = useScroll({ target: ref, offset: ["start start", "end end"] });
  const p0 = useTransform(scrollYProgress, (v) => clamp(v * n - 0));
  const p1 = useTransform(scrollYProgress, (v) => clamp(v * n - 1));
  const p2 = useTransform(scrollYProgress, (v) => clamp(v * n - 2));
  const p3 = useTransform(scrollYProgress, (v) => clamp(v * n - 3));
  const locals = [p0, p1, p2, p3];
  const [active, setActive] = useState(0);
  useMotionValueEvent(scrollYProgress, "change", (v) => setActive(Math.min(n - 1, Math.floor(v * n))));

  const open = (i: number) => {
    const el = ref.current;
    if (!el) return;
    const top = el.getBoundingClientRect().top + window.scrollY;
    const target = top + ((el.offsetHeight - window.innerHeight) * (i + 0.5)) / n;
    if (lenis) lenis.scrollTo(target, { duration: 1.2 });
    else window.scrollTo({ top: target, behavior: "smooth" });
  };

  return (
    <div ref={ref} className="relative" style={{ height: `${n * 85}vh` }}>
      <div className="sticky top-[4.5rem] mx-auto flex h-[calc(100dvh-4.5rem)] max-w-page gap-3 px-8 pb-8 pt-4">
        {columns.map((column, i) => (
          <Pillar key={column.key} column={column} index={i} open={i === active} p={locals[i]} onOpen={() => open(i)} />
        ))}
      </div>
    </div>
  );
}

/** Phones and reduced motion: each column as its own card, its widget playing once when seen. */
function Card({ column, index }: { column: Column; index: number }) {
  const ref = useRef<HTMLLIElement>(null);
  const seen = useInView(ref, { once: true, margin: "0px 0px -20% 0px" });
  const reduce = useReducedMotion();
  const p = useMotionValue(0);
  useEffect(() => {
    if (!seen) return;
    if (reduce) return void p.set(1);
    const run = animate(p, 1, { duration: 2.2, ease: "easeInOut" });
    return () => run.stop();
  }, [seen, reduce, p]);
  const Widget = WIDGETS[column.key];
  return (
    <li ref={ref} className="metal flex flex-col gap-5 p-6">
      <span className="flex items-center justify-between">
        <span className="shout text-[1.8rem]">{column.label}</span>
        <span className="font-mono text-[0.72rem] text-text-muted">{String(index + 1).padStart(3, "0")}</span>
      </span>
      <span className="headline silver text-[1.7rem]">{column.title}</span>
      <span className="leading-relaxed text-text-muted">{column.body}</span>
      <Widget p={p} />
    </li>
  );
}

export function Colonnade({ columns }: { columns: Column[] }) {
  const wide = useMedia("(min-width: 1024px) and (prefers-reduced-motion: no-preference)");
  if (wide) return <Pinned columns={columns} />;
  return (
    <ol className="mx-auto flex max-w-page flex-col gap-4 px-5 pb-24 sm:px-8">
      {columns.map((column, i) => (
        <Card key={column.key} column={column} index={i} />
      ))}
    </ol>
  );
}
