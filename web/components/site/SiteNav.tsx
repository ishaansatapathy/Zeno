"use client";

import { useMotionValueEvent, useScroll } from "motion/react";
import { useTranslations } from "next-intl";
import Link from "next/link";
import { useState } from "react";

import { cn } from "@/components/ui/cn";

import { SiteButton } from "./SiteButton";
import { Wordmark } from "./Wordmark";

const anchors = [
  ["#how", "howItWorks"],
  ["#money", "money"],
  ["#live", "live"],
  ["#students", "students"],
] as const;

/** Transparent over the hero; a paper bar with a hairline once the page moves. */
export function SiteNav() {
  const t = useTranslations("site.nav");
  const { scrollY } = useScroll();
  const [lifted, setLifted] = useState(false);
  useMotionValueEvent(scrollY, "change", (y) => setLifted(y > 12));

  return (
    <header
      className={cn(
        "sticky top-0 z-40 transition-[background-color,border-color] duration-300",
        lifted ? "border-b border-border bg-bg/90 backdrop-blur-[6px]" : "border-b border-transparent",
      )}
    >
      <div className="mx-auto flex h-[4.5rem] max-w-page items-center justify-between gap-6 px-5 sm:px-8">
        <Link href="/" aria-label={t("home")} className="text-text">
          <Wordmark />
        </Link>
        <nav aria-label="Main" className="hidden items-center gap-8 lg:flex">
          {anchors.map(([href, key]) => (
            <a key={href} href={href} className="text-[0.92rem] text-text-muted transition-colors hover:text-text">
              {t(key)}
            </a>
          ))}
        </nav>
        <div className="flex items-center gap-2 sm:gap-4">
          <Link href="/sign-in" className="hidden text-[0.92rem] text-text-muted transition-colors hover:text-text sm:inline">
            {t("signIn")}
          </Link>
          <SiteButton href="/sign-up" arrow className="h-10 px-5 text-[0.9rem]">
            {t("start")}
          </SiteButton>
        </div>
      </div>
    </header>
  );
}
