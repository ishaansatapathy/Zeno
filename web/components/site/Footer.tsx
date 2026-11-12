import Link from "next/link";

import { footer } from "@/content/site";

import { Logo } from "./Logo";

export function Footer() {
  return (
    <footer className="border-t border-cream/10">
      <div className="mx-auto grid max-w-page gap-12 px-4 py-14 sm:px-6 lg:grid-cols-[minmax(0,1fr)_auto] lg:px-8">
        <div>
          <Logo className="text-2xl" />
          <p className="caps mt-4 max-w-[22rem] text-[0.78rem] tracking-[0.1em] text-dust">{footer.tagline}</p>
        </div>
        <div className="flex gap-16 sm:gap-24">
          {footer.columns.map((col) => (
            <nav key={col.title} aria-label={col.title}>
              <h2 className="caps text-[0.72rem] tracking-[0.16em] text-violet-hi">{col.title}</h2>
              <ul className="mt-4 space-y-2.5">
                {col.links.map((l) => (
                  <li key={l.href}>
                    {l.href.startsWith("#") ? (
                      <a href={l.href} className="caps text-[0.9rem] text-cream transition-colors hover:text-spray">
                        {l.label}
                      </a>
                    ) : (
                      <Link href={l.href} prefetch={false} className="caps text-[0.9rem] text-cream transition-colors hover:text-spray">
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
      <div className="mx-auto flex max-w-page flex-wrap justify-between gap-3 px-4 pb-10 sm:px-6 lg:px-8">
        <p className="caps text-[0.72rem] tracking-[0.14em] text-dust">© {new Date().getFullYear()} Zeno</p>
        <p className="caps text-[0.72rem] tracking-[0.14em] text-dust">{footer.made}</p>
      </div>
    </footer>
  );
}
