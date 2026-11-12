import Link from "next/link";

import { links, nav } from "@/content/site";

import { ArrowUpRight } from "./frame";

/** The wordmark: an orange tile with a play mark cut into it, and the name. */
export function Mark({ className = "" }: { className?: string }) {
  return (
    <span className={`inline-flex items-center gap-2.5 ${className}`}>
      <svg viewBox="0 0 20 20" aria-hidden="true" focusable="false" className="size-[1.15em]">
        <rect width="20" height="20" fill="#ff5b14" />
        <path d="M7.5 5.5v9l7-4.5-7-4.5Z" fill="#050505" />
      </svg>
      <span className="font-sans text-[1.05em] font-semibold uppercase tracking-[0.22em]">Zeno</span>
    </span>
  );
}

/**
 * A row of boxes along the top, like the header of a drawing: the name, the sections, logging in,
 * and the one orange box that starts everything. Each box fills from the bottom on hover.
 */
export function Nav() {
  const cell = "caps fillup flex h-full items-center px-5 text-paper/85 border-l border-line";
  return (
    <header className="fixed inset-x-0 top-0 z-50 h-12 border-b border-line bg-ink/85 backdrop-blur-md">
      <div className="flex h-full items-stretch">
        <Link href="/" aria-label="Zeno, home" className="flex items-center px-4 text-[0.8rem] text-paper sm:px-5 lg:w-56">
          <Mark />
        </Link>
        <nav aria-label="Sections" className="ml-auto hidden h-full lg:block">
          <ul className="flex h-full">
            {nav.map((item) => (
              <li key={item.href} className="h-full">
                <a href={item.href} className={`${cell} xl:px-8`}>
                  {item.label}
                </a>
              </li>
            ))}
          </ul>
        </nav>
        <Link href={links.login} prefetch={false} className={`${cell} ml-auto hidden sm:flex lg:ml-0`}>
          Log in
        </Link>
        <Link
          href={links.start}
          prefetch={false}
          className="caps fillup fillup-paper ml-auto flex items-center gap-6 bg-signal px-5 text-ink sm:ml-0 sm:gap-10 lg:w-56 lg:justify-between"
        >
          Start teaching
          <ArrowUpRight />
        </Link>
      </div>
    </header>
  );
}
