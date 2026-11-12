import Link from "next/link";

import { ArrowRight, Drips } from "./doodles";

/**
 * The call to action: a slab of cream (or smoke) on a hard violet shadow that paint is running
 * off. On hover the face lifts off its shadow and the drips run longer.
 */
export function DripButton({
  href,
  children,
  tone = "cream",
  size = "md",
  drips = true,
  className = "",
}: {
  href: string;
  children: string;
  tone?: "cream" | "smoke";
  size?: "sm" | "md" | "lg";
  drips?: boolean;
  className?: string;
}) {
  const face =
    tone === "cream" ? "bg-cream text-ink" : "bg-smoke text-cream border border-cream/15";
  const pad = { sm: "px-3.5 py-2 text-[0.95rem]", md: "px-5 py-3 text-lg", lg: "px-6 py-3.5 text-xl" }[size];
  const shadow = tone === "cream" ? "bg-violet text-violet" : "bg-cream/30 text-cream/30";
  return (
    <Link href={href} prefetch={false} className={`group relative inline-block shrink-0 select-none ${className}`}>
      <span aria-hidden="true" className={`absolute inset-0 translate-x-[5px] translate-y-[5px] ${shadow}`}>
        {drips ? <Drips className="absolute left-0 top-full h-4 w-full" /> : null}
      </span>
      <span
        className={`shout relative flex items-center gap-2.5 transition-transform duration-300 ease-out group-hover:-translate-x-[2px] group-hover:-translate-y-[2px] group-active:translate-x-[3px] group-active:translate-y-[3px] ${face} ${pad}`}
      >
        {children}
        <ArrowRight className="size-[1.1em] transition-transform duration-300 group-hover:translate-x-1" />
      </span>
    </Link>
  );
}
