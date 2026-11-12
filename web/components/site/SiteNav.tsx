"use client";

import { useMotionValueEvent, useScroll } from "motion/react";
import { useTranslations } from "next-intl";
import Link from "next/link";
import { useState } from "react";

import { cn } from "@/components/ui/cn";

import { SiteButton } from "./SiteButton";
import { Wordmark } from "./Wordmark";

const anchors = [
  ["#how", "how"],
  ["#who", "who"],
  ["#product", "product"],
  ["#guarantees", "guarantees"],
] as const;

/** The mark on the left, a pill of links in the middle, the status and the one button on the right. */
export function SiteNav() {
  const t = useTranslations("site.nav");
  const { scrollY } = useScroll();
  const [lifted, setLifted] = useState(false);
  useMotionValueEvent(scrollY, "change", (y) => setLifted(y > 24));

  return (
    <header className="fixed inset-x-0 top-0 z-40">
      <div
        className={cn(
          "absolute inset-0 -z-10 border-b transition-[background-color,border-color,backdrop-filter] duration-500",
          lifted ? "border-white/[0.06] bg-[#050505]/75 backdrop-blur-xl" : "border-transparent",
        )}
      />
      <div className="mx-auto flex h-[4.5rem] max-w-page items-center justify-between gap-6 px-5 sm:px-8">
        <Link href="/" aria-label={t("home")} className="text-text">
          <Wordmark />
        </Link>
        <nav aria-label="Main" className="hidden items-center rounded-full border border-white/10 bg-white/[0.03] p-1 lg:flex">
          {anchors.map(([href, key]) => (
            <a key={href} href={href} className="rounded-full px-4 py-1.5 text-[0.88rem] text-text-muted transition-colors hover:bg-white/[0.06] hover:text-text">
              {t(key)}
            </a>
          ))}
        </nav>
        <div className="flex items-center gap-3">
          <span className="hidden items-center gap-2 rounded-full border border-white/10 px-3 py-1.5 font-mono text-[0.72rem] text-text-muted md:inline-flex">
            <span className="h-1.5 w-1.5 rounded-full bg-success shadow-[0_0_8px_rgb(var(--success))]" />
            {t("status")}
          </span>
          <Link href="/sign-in" className="hidden text-[0.88rem] text-text-muted transition-colors hover:text-text sm:inline">
            {t("signIn")}
          </Link>
          <SiteButton href="/sign-up" size="sm">
            {t("start")}
          </SiteButton>
        </div>
      </div>
    </header>
  );
}
