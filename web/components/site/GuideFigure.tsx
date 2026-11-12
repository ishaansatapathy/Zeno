"use client";

import { m, useReducedMotion } from "motion/react";
import { useEffect, useState } from "react";

import { cn } from "@/components/ui/cn";

/**
 * The guide on the page: rises out from behind a line the first time it is seen, breathes a
 * little, blinks now and then, and says one short thing in a speech bubble. The SVGs come from
 * our own server (fixed options, no user input), so injecting them is safe.
 */
export function GuideFigure({
  face,
  blink,
  says,
  side,
  alt,
  className,
}: {
  face: string;
  blink: string;
  says?: string;
  side: "left" | "right";
  alt: string;
  className?: string;
}) {
  const reduce = useReducedMotion();
  const [closed, setClosed] = useState(false);

  useEffect(() => {
    if (reduce) return;
    let open: ReturnType<typeof setTimeout>;
    const loop = setInterval(
      () => {
        setClosed(true);
        open = setTimeout(() => setClosed(false), 140);
      },
      3_800 + Math.round(Math.random() * 1_600),
    );
    return () => {
      clearInterval(loop);
      clearTimeout(open);
    };
  }, [reduce]);

  return (
    <div className={cn("flex items-end gap-3", side === "right" && "flex-row-reverse", className)}>
      <div className="relative h-28 w-28 shrink-0 overflow-hidden border-b border-text sm:h-32 sm:w-32">
        <m.div
          role="img"
          aria-label={alt}
          className="absolute inset-x-0 bottom-0 h-full"
          initial={{ y: "70%" }}
          whileInView={{ y: "6%" }}
          viewport={{ once: true, margin: "0px 0px -10% 0px" }}
          transition={{ type: "spring", stiffness: 140, damping: 16, delay: 0.1 }}
        >
          <m.div
            className="relative h-full w-full"
            animate={reduce ? undefined : { y: [0, -3, 0] }}
            transition={{ duration: 4.2, repeat: Infinity, ease: "easeInOut" }}
          >
            <div className="absolute inset-0 [&>svg]:h-full [&>svg]:w-full" dangerouslySetInnerHTML={{ __html: face }} />
            <div
              aria-hidden="true"
              className={cn("absolute inset-0 [&>svg]:h-full [&>svg]:w-full", closed ? "opacity-100" : "opacity-0")}
              dangerouslySetInnerHTML={{ __html: blink }}
            />
          </m.div>
        </m.div>
      </div>
      {says && (
        <m.p
          className={cn(
            "relative mb-10 max-w-[15rem] rounded-card border border-text bg-surface px-4 py-3 text-sm leading-snug shadow-[3px_3px_0_0_rgb(var(--text))]",
            side === "left" ? "rounded-bl-none" : "rounded-br-none",
          )}
          initial={{ opacity: 0, scale: 0.9, y: 8 }}
          whileInView={{ opacity: 1, scale: 1, y: 0 }}
          viewport={{ once: true, margin: "0px 0px -10% 0px" }}
          transition={{ type: "spring", stiffness: 260, damping: 20, delay: 0.45 }}
        >
          {says}
        </m.p>
      )}
    </div>
  );
}
