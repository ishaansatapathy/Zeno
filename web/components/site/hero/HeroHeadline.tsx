"use client";

import { m, useScroll, useTransform } from "motion/react";
import { useRef } from "react";

import { Emphasis } from "../Emphasis";

const ease = [0.22, 1, 0.36, 1] as const;

/**
 * Two lines, said simply. On arrival each word rises out of its own baseline; as the page scrolls
 * away the first line lifts faster than the second, so the sentence pulls apart a little, like
 * a page turning, instead of just sliding off.
 */
export function HeroHeadline({ line1, line2 }: { line1: string; line2: string }) {
  const ref = useRef<HTMLHeadingElement>(null);
  const { scrollYProgress } = useScroll({ target: ref, offset: ["start 30%", "end start"] });
  const y1 = useTransform(scrollYProgress, [0, 1], [0, -90]);
  const y2 = useTransform(scrollYProgress, [0, 1], [0, -30]);
  const fade = useTransform(scrollYProgress, [0.4, 1], [1, 0]);

  const words = line1.split(" ");
  return (
    <h1 ref={ref} className="display text-[clamp(2.9rem,8.6vw,8.4rem)] [text-wrap:balance]">
      <m.span className="block" style={{ y: y1, opacity: fade }}>
        {words.map((word, i) => (
          <span key={i} className="inline-block overflow-hidden pb-[0.12em] align-bottom">
            <m.span
              className="inline-block"
              initial={{ y: "105%" }}
              animate={{ y: "0%" }}
              transition={{ duration: 1, delay: 0.1 + i * 0.08, ease }}
            >
              {word}
              {i < words.length - 1 && " "}
            </m.span>
          </span>
        ))}
      </m.span>
      <m.span
        className="block"
        style={{ y: y2, opacity: fade }}
        initial={{ opacity: 0, y: 24 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 1, delay: 0.45, ease }}
      >
        <Emphasis>{line2}</Emphasis>
      </m.span>
    </h1>
  );
}
