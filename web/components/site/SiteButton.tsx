import Link from "next/link";
import type { ReactNode } from "react";

import { cn } from "@/components/ui/cn";

/**
 * The site's two buttons, both links: a solid ink pill for the one thing to do, and an outlined
 * one for the alternative. The arrow leans forward on hover; nothing else moves.
 */
export function SiteButton({
  href,
  children,
  variant = "ink",
  arrow = false,
  className,
}: {
  href: string;
  children: ReactNode;
  variant?: "ink" | "outline" | "accent";
  arrow?: boolean;
  className?: string;
}) {
  return (
    <Link
      href={href}
      className={cn(
        "group inline-flex h-12 items-center justify-center gap-2 rounded-full px-6 text-[0.95rem] font-medium transition-colors duration-200",
        variant === "ink" && "bg-primary text-on-primary hover:bg-primary-hover",
        variant === "outline" && "border border-text text-text hover:bg-text hover:text-bg",
        variant === "accent" && "border border-text bg-accent text-on-accent shadow-[3px_3px_0_0_rgb(var(--text))] hover:translate-x-[1px] hover:translate-y-[1px] hover:shadow-[2px_2px_0_0_rgb(var(--text))]",
        className,
      )}
    >
      {children}
      {arrow && (
        <svg
          aria-hidden="true"
          viewBox="0 0 16 16"
          className="h-4 w-4 transition-transform duration-300 ease-out group-hover:translate-x-1"
        >
          <path d="M2 8h11M9 3.5 13.5 8 9 12.5" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      )}
    </Link>
  );
}
