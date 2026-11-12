"use client";

import { AnimatePresence, m, useReducedMotion, useScroll, useTransform } from "motion/react";
import { useEffect, useRef, useState } from "react";

/** A small tilted metal tag set into the line of type, its word changing every few seconds. */
function Chip({ words, offset, tilt }: { words: string[]; offset: number; tilt: number }) {
  const reduce = useReducedMotion();
  const [i, setI] = useState(offset % words.length);
  useEffect(() => {
    if (reduce) return;
    const timer = setInterval(() => setI((v) => (v + 1) % words.length), 2_400);
    return () => clearInterval(timer);
  }, [reduce, words.length]);
  return (
    <span
      aria-hidden="true"
      className="metal relative mx-[0.1em] inline-grid h-[0.62em] w-[1.75em] translate-y-[-0.12em] place-items-center overflow-hidden align-middle"
      style={{ rotate: `${tilt}deg` }}
    >
      <AnimatePresence mode="popLayout" initial={false}>
        <m.span
          key={words[i]}
          initial={{ y: "110%" }}
          animate={{ y: "0%" }}
          exit={{ y: "-110%" }}
          transition={{ duration: 0.55, ease: [0.22, 1, 0.36, 1] }}
          className="whitespace-nowrap font-mono text-[0.11em] font-normal normal-case tracking-normal text-text"
        >
          {words[i]}
        </m.span>
      </AnimatePresence>
    </span>
  );
}

/**
 * The statement: two sentences in big condensed capitals, with metal tags set into the lines.
 * As the page passes, the lines drift a little in opposite directions.
 */
export function Statement({ lines, chips }: { lines: [string, string, string, string]; chips: string[] }) {
  const ref = useRef<HTMLDivElement>(null);
  const { scrollYProgress } = useScroll({ target: ref, offset: ["start end", "end start"] });
  const left = useTransform(scrollYProgress, [0, 1], ["4%", "-4%"]);
  const right = useTransform(scrollYProgress, [0, 1], ["-4%", "4%"]);
  const half = Math.ceil(chips.length / 2);
  return (
    <div ref={ref} className="overflow-hidden py-24 lg:py-36">
      <p className="sr-only">{lines.join(" ")}</p>
      <div aria-hidden="true" className="shout mx-auto flex max-w-page flex-col px-5 text-[clamp(3rem,10.5vw,9.5rem)] sm:px-8">
        <m.span className="block whitespace-nowrap" style={{ x: left }}>
          <span className="silver">{lines[0]}</span> <Chip words={chips.slice(0, half)} offset={0} tilt={-4} />
        </m.span>
        <m.span className="block whitespace-nowrap pl-[0.9em]" style={{ x: right }}>
          <span className="silver">{lines[1]}</span>
        </m.span>
        <m.span className="mt-[0.12em] block whitespace-nowrap text-right" style={{ x: left }}>
          <Chip words={chips.slice(half)} offset={1} tilt={3} /> <span className="silver">{lines[2]}</span>
        </m.span>
        <m.span className="block whitespace-nowrap text-right pr-[0.6em]" style={{ x: right }}>
          <span className="silver">{lines[3]}</span>
        </m.span>
      </div>
    </div>
  );
}
