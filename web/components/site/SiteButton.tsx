import Link from "next/link";
import type { ReactNode } from "react";

import { cn } from "@/components/ui/cn";

/**
 * The site's buttons, all links: a polished silver pill for the one thing to do, and a dark glass
 * one for the alternative. The arrow leans forward on hover; nothing else moves.
 */
export function SiteButton({
  href,
  children,
  variant = "silver",
  arrow = false,
  size = "md",
  className,
}: {
  href: string;
  children: ReactNode;
  variant?: "silver" | "glass" | "ink";
  arrow?: boolean;
  size?: "sm" | "md";
  className?: string;
}) {
  return (
    <Link
      href={href}
      className={cn(
        "group inline-flex items-center justify-center gap-2 rounded-full font-medium transition-[background-color,box-shadow,color] duration-300",
        size === "md" ? "h-12 px-6 text-[0.95rem]" : "h-9 px-4 text-[0.85rem]",
        variant === "silver" && "plate hover:brightness-[1.04]",
        variant === "glass" && "border border-white/12 bg-white/[0.04] text-text hover:bg-white/[0.08]",
        variant === "ink" && "bg-[#050505] text-[#f4f4f5] hover:bg-[#18181b]",
        className,
      )}
    >
      {children}
      {arrow && (
        <svg aria-hidden="true" viewBox="0 0 16 16" className="h-4 w-4 transition-transform duration-300 ease-out group-hover:translate-x-0.5">
          <path d="M2 8h11M9 3.5 13.5 8 9 12.5" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      )}
    </Link>
  );
}
