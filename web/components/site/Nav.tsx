"use client";

import { useMotionValueEvent, useScroll } from "motion/react";
import Link from "next/link";
import { useState } from "react";

import { links, nav } from "@/content/site";

import { Container } from "./frame";

/** The wordmark: a small paper tile with a play mark cut out of it, and the name. */
export function Mark({ className = "" }: { className?: string }) {
  return (
    <span className={`inline-flex items-center gap-2 ${className}`}>
      <svg viewBox="0 0 20 20" aria-hidden="true" focusable="false" className="size-[1.25em]">
        <rect width="20" height="20" rx="5" fill="#ededed" />
        <path d="M8 6.2v7.6l6-3.8-6-3.8Z" fill="#09090b" />
      </svg>
      <span className="text-[1.05em] font-semibold tracking-[-0.02em]">Zeno</span>
    </span>
  );
}

/**
 * A plain bar along the top. It starts clear over the hero and, once the page moves, settles into
 * a frosted strip with a hairline under it.
 */
export function Nav() {
  const { scrollY } = useScroll();
  const [stuck, setStuck] = useState(false);
  useMotionValueEvent(scrollY, "change", (y) => setStuck(y > 8));

  return (
    <header className={`fixed inset-x-0 top-0 z-50 border-b transition-[background-color,border-color] duration-300 ${stuck ? "border-line bg-ink/90 backdrop-blur-md" : "border-transparent"}`}>
      <Container className="flex h-14 items-center gap-8">
        <Link href="/" aria-label="Zeno, home" className="text-[0.95rem]">
          <Mark />
        </Link>
        <nav aria-label="Sections" className="mx-auto hidden md:block">
          <ul className="flex items-center gap-1">
            {nav.map((item) => (
              <li key={item.href}>
                <a href={item.href} className="rounded-md px-3 py-2 text-[0.84rem] text-mute transition-colors hover:text-paper">
                  {item.label}
                </a>
              </li>
            ))}
          </ul>
        </nav>
        <div className="ml-auto flex items-center gap-2 md:ml-0">
          <Link href={links.login} prefetch={false} className="hidden rounded-md px-3 py-2 text-[0.84rem] text-mute transition-colors hover:text-paper sm:block">
            Log in
          </Link>
          <Link href={links.start} prefetch={false} className="btn btn-solid h-8 px-3.5 text-[0.82rem]">
            Start teaching
          </Link>
        </div>
      </Container>
    </header>
  );
}
