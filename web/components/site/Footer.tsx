import Link from "next/link";

import { footer } from "@/content/site";

import { Container } from "./frame";

/** The name, large, in the serif's italic and polished silver. Drawn as a picture: it is decoration. */
function ChromeName({ className = "" }: { className?: string }) {
  return (
    <svg viewBox="0 0 1000 300" aria-hidden="true" focusable="false" className={className}>
      <defs>
        <linearGradient id="footer-chrome" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0.12" stopColor="#ffffff" />
          <stop offset="0.5" stopColor="#e3e7eb" />
          <stop offset="0.62" stopColor="#a9b1bb" />
          <stop offset="1" stopColor="#f2f4f6" />
        </linearGradient>
      </defs>
      <text x="500" y="262" textAnchor="middle" fill="url(#footer-chrome)" fontFamily="var(--font-serif)" fontStyle="italic" fontSize="340" letterSpacing="-6">
        Zeno
      </text>
    </svg>
  );
}

/** The links, a thin row under them, and the name across the foot of the page. */
export function Footer() {
  return (
    <footer className="overflow-hidden border-t border-line">
      <Container className="grid gap-14 py-16 lg:grid-cols-[minmax(0,1fr)_auto] lg:py-20">
        <p className="label max-w-[16rem]">{footer.made}</p>
        <div className="flex gap-16 sm:gap-24">
          {footer.columns.map((col) => (
            <nav key={col.title} aria-label={col.title}>
              <h2 className="label flex items-center gap-2">
                <span aria-hidden="true" className="size-1 bg-silver" />
                {col.title}
              </h2>
              <ul className="mt-5 space-y-2.5">
                {col.links.map((l) => (
                  <li key={l.href}>
                    {l.href.startsWith("#") ? (
                      <a href={l.href} className="caps text-paper/85 transition-colors hover:text-silver-hi">
                        {l.label}
                      </a>
                    ) : (
                      <Link href={l.href} prefetch={false} className="caps text-paper/85 transition-colors hover:text-silver-hi">
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
      <Container className="flex flex-wrap items-center justify-between gap-4 border-t border-line py-5">
        <p className="label">
          © {new Date().getFullYear()} {footer.rights}
        </p>
        <a href="#main" className="caps fillup fillup-ink bg-silver px-4 py-2 text-ink">
          {footer.top} ↑
        </a>
      </Container>
      <ChromeName className="mx-auto -mb-[3vw] block w-full max-w-[110rem] px-2" />
    </footer>
  );
}
