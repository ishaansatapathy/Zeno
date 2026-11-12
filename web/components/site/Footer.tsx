import Link from "next/link";

import { footer } from "@/content/site";

import { Container } from "./frame";
import { Mark } from "./Nav";

/** Links in columns, a thin row under them, and the name set large and nearly dark. */
export function Footer() {
  return (
    <footer className="overflow-hidden border-t border-line">
      <Container className="grid gap-12 py-16 lg:grid-cols-[minmax(0,1fr)_auto]">
        <div>
          <Mark className="text-[0.95rem]" />
          <p className="mt-4 max-w-[18rem] text-[0.88rem] leading-relaxed text-mute">{footer.made}.</p>
        </div>
        <div className="flex gap-16 sm:gap-24">
          {footer.columns.map((col) => (
            <nav key={col.title} aria-label={col.title}>
              <h2 className="label">{col.title}</h2>
              <ul className="mt-4 space-y-2.5">
                {col.links.map((l) => (
                  <li key={l.href}>
                    {l.href.startsWith("#") ? (
                      <a href={l.href} className="text-[0.9rem] text-mute transition-colors hover:text-paper">
                        {l.label}
                      </a>
                    ) : (
                      <Link href={l.href} prefetch={false} className="text-[0.9rem] text-mute transition-colors hover:text-paper">
                        {l.label}
                      </Link>
                    )}
                  </li>
                ))}
              </ul>
            </nav>
          ))}
        </div>
      </Container>
      <Container className="flex items-center justify-between border-t border-line py-5">
        <p className="label">
          © {new Date().getFullYear()} {footer.rights}
        </p>
        <a href="#main" className="label transition-colors hover:text-paper">
          {footer.top} ↑
        </a>
      </Container>
      {/* The name as a picture, not text: it is decoration, set nearly as dark as the page. */}
      <svg viewBox="0 0 1000 250" aria-hidden="true" focusable="false" className="mx-auto -mb-[2vw] block w-full max-w-[90rem] select-none">
        <text x="500" y="262" textAnchor="middle" fill="#16171a" fontFamily="var(--font-sans)" fontWeight="500" fontSize="330" letterSpacing="-20">
          Zeno
        </text>
      </svg>
    </footer>
  );
}
