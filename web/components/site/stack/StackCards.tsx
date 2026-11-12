"use client";

import { m, useScroll, useTransform, type MotionValue } from "motion/react";
import { useRef } from "react";

export type Guarantee = { big: string; body: string };

function Sheet({ item, index, total, progress }: { item: Guarantee; index: number; total: number; progress: MotionValue<number> }) {
  // Each sheet shrinks a little as the ones after it slide over, so the pile reads as depth.
  const scale = useTransform(progress, [index / total, 1], [1, 1 - (total - 1 - index) * 0.045]);
  const dim = useTransform(progress, [index / total, 1], [1, 1 - (total - 1 - index) * 0.18]);
  return (
    <div className="sticky h-[78vh]" style={{ top: `calc(6.5rem + ${index * 1.4}rem)` }}>
      <m.article
        style={{ scale, opacity: dim, transformOrigin: "50% 0%" }}
        className="metal grid min-h-[22rem] gap-8 p-8 sm:p-10 lg:grid-cols-[1.25fr_1fr] lg:items-end"
      >
        <div className="flex flex-col gap-6">
          <span className="font-mono text-[0.72rem] text-text-muted">
            {String(index + 1).padStart(2, "0")} / {String(total).padStart(2, "0")}
          </span>
          <h3 className="shout silver text-[clamp(3.4rem,8vw,7rem)]">{item.big}</h3>
        </div>
        <p className="max-w-md text-[1.1rem] leading-relaxed text-text-muted">{item.body}</p>
      </m.article>
    </div>
  );
}

/** Sheets that pin one after another and pile up as the page scrolls. */
export function StackCards({ items }: { items: Guarantee[] }) {
  const ref = useRef<HTMLDivElement>(null);
  const { scrollYProgress } = useScroll({ target: ref, offset: ["start start", "end end"] });
  return (
    <div ref={ref} className="mx-auto max-w-page px-5 sm:px-8">
      {items.map((item, i) => (
        <Sheet key={item.big} item={item} index={i} total={items.length} progress={scrollYProgress} />
      ))}
    </div>
  );
}
