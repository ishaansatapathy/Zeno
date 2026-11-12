import Link from "next/link";

import { links, nav } from "@/content/site";

import { DripButton } from "./DripButton";
import { Person } from "./doodles";
import { Logo } from "./Logo";

export function Nav() {
  return (
    <header className="fixed inset-x-0 top-0 z-50 border-b border-cream/10 bg-ink/80 backdrop-blur-md">
      <div className="mx-auto flex h-16 max-w-page items-center gap-6 px-4 sm:px-6 lg:px-8">
        <Logo className="text-base" />
        <nav aria-label="Sections" className="ml-auto hidden md:block">
          <ul className="flex items-center gap-7">
            {nav.map((item) => (
              <li key={item.href}>
                <a href={item.href} className="caps text-[0.82rem] text-cream/80 transition-colors hover:text-spray">
                  {item.label}
                </a>
              </li>
            ))}
          </ul>
        </nav>
        <Link
          href={links.login}
          prefetch={false}
          className="caps ml-auto inline-flex items-center gap-1.5 text-[0.82rem] text-cream/80 transition-colors hover:text-spray md:ml-4"
        >
          Log in
          <Person className="size-4 text-violet-hi" />
        </Link>
        <DripButton href={links.start} size="sm" className="mb-1">
          Start teaching
        </DripButton>
      </div>
    </header>
  );
}
