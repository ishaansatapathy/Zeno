import Link from "next/link";

import { footer } from "@/content/site";

import { Mark } from "./Nav";

/** Links in columns, and under them the name, as wide as the page and cut off by its edge. */
export function Footer() {
  return (
    <footer className="overflow-hidden border-t border-line">
      <div className="mx-auto grid max-w-page gap-12 px-4 py-14 sm:px-6 lg:grid-cols-[minmax(0,1fr)_auto] lg:px-10">
        <div>
          <Mark className="text-[0.85rem]" />
          <p className="label mt-5">{footer.made}</p>
        </div>
        <div className="flex gap-16 sm:gap-24">
          {footer.columns.map((col) => (
            <nav key={col.title} aria-label={col.title}>
              <h2 className="label flex items-center gap-2">
                <span className="size-1 bg-signal" />
                {col.title}
              </h2>
              <ul className="mt-5 space-y-2.5">
                {col.links.map((l) => (
                  <li key={l.href}>
                    {l.href.startsWith("#") ? (
                      <a href={l.href} className="caps text-paper/85 transition-colors hover:text-signal">
                        {l.label}
                      </a>
                    ) : (
                      <Link href={l.href} prefetch={false} className="caps text-paper/85 transition-colors hover:text-signal">
                        {l.label}
                      </Link>
                    )}
                  </li>
                ))}
              </ul>
            </nav>
          ))}
        </div>
      </div>
      <div className="mx-auto flex max-w-page flex-wrap items-center justify-between gap-4 border-t border-line px-4 py-5 sm:px-6 lg:px-10">
        <p className="label">
          © {new Date().getFullYear()} {footer.rights}
        </p>
        <a href="#main" className="caps fillup fillup-paper bg-signal px-4 py-2 text-ink">
          {footer.top} ↑
        </a>
      </div>
      <p aria-hidden="true" className="-mb-[5.5vw] select-none text-center font-cond text-[33vw] uppercase leading-[0.8] tracking-[-0.01em] text-paper">
        Zeno
      </p>
    </footer>
  );
}
