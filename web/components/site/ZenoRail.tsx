"use client";

import { m, useScroll, useTransform } from "motion/react";
import { useTranslations } from "next-intl";
import { useEffect, useState } from "react";

import { cn } from "@/components/ui/cn";

const SECTIONS = [
  ["top", "hero"],
  ["how", "pipeline"],
  ["money", "money"],
  ["live", "live"],
  ["students", "students"],
  ["begin", "close"],
] as const;

/**
 * The line itself, on wide screens: down the left edge, filling as the page is read, with a node
 * for every section. Each node is a link; the one you are in is saffron.
 */
export function ZenoRail() {
  const t = useTranslations("site.rail");
  const { scrollYProgress } = useScroll();
  const fill = useTransform(scrollYProgress, [0, 1], ["0%", "100%"]);
  const [active, setActive] = useState<string>("top");

  useEffect(() => {
    const io = new IntersectionObserver(
      (entries) => entries.forEach((entry) => entry.isIntersecting && setActive(entry.target.id)),
      { rootMargin: "-45% 0px -50% 0px" },
    );
    SECTIONS.forEach(([id]) => {
      const el = document.getElementById(id);
      if (el) io.observe(el);
    });
    return () => io.disconnect();
  }, []);

  return (
    <nav aria-label={t("label")} className="fixed left-6 top-1/2 z-30 hidden -translate-y-1/2 min-[1400px]:block">
      <div className="relative flex flex-col gap-7 py-1">
        <span aria-hidden="true" className="absolute bottom-0 left-[5px] top-0 w-px bg-border-strong" />
        <m.span aria-hidden="true" className="absolute left-[4.5px] top-0 w-[2px] origin-top bg-text" style={{ height: fill }} />
        {SECTIONS.map(([id, key]) => {
          const on = active === id;
          return (
            <a key={id} href={`#${id}`} aria-current={on ? "location" : undefined} className="group relative flex items-center">
              <span
                className={cn(
                  "relative z-10 h-[11px] w-[11px] rounded-full border border-text transition-all duration-300",
                  on ? "scale-125 bg-accent" : "bg-bg group-hover:bg-accent/60",
                )}
              />
              <span
                className={cn(
                  "label absolute left-6 whitespace-nowrap rounded-full bg-bg/90 px-2 py-0.5 transition-opacity duration-200",
                  "opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100", on && "text-text",
                )}
              >
                {t(key)}
              </span>
            </a>
          );
        })}
      </div>
    </nav>
  );
}
