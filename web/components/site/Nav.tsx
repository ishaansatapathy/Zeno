import Link from "next/link";

import { links, nav } from "@/content/site";

import { ArrowUpRight } from "./frame";
import { Mark } from "./Mark";

/**
 * A bar that floats just inside the page's edges: the name, then a row of boxes for the sections,
 * logging in, and the one silver box that starts everything. Each box fills from the bottom on
 * hover.
 */
export function Nav() {
  const cell = "caps fillup flex h-full items-center whitespace-nowrap border-l border-line px-5 text-paper/80 xl:px-7";
  return (
    <header className="fixed inset-x-2 top-2 z-50 h-12 border border-line bg-ink/80 backdrop-blur-md sm:inset-x-3 sm:top-3">
      <div className="flex h-full items-stretch">
        <Link href="/" aria-label="Zeno, home" className="flex items-center px-4 text-[0.82rem] sm:px-5">
          <Mark />
        </Link>
        <nav aria-label="Sections" className="ml-auto hidden h-full lg:block">
          <ul className="flex h-full">
            {nav.map((item) => (
              <li key={item.href} className="h-full">
                <a href={item.href} className={cell}>
                  {item.label}
                </a>
              </li>
            ))}
          </ul>
        </nav>
        <Link href={links.login} prefetch={false} className={`${cell} ml-auto hidden sm:flex lg:ml-0`}>
          Log in
        </Link>
        <Link href={links.start} prefetch={false} className="caps fillup fillup-ink ml-auto flex items-center gap-6 whitespace-nowrap bg-silver px-5 text-ink sm:ml-0 lg:gap-10">
          Start teaching
          <ArrowUpRight />
        </Link>
      </div>
    </header>
  );
}
