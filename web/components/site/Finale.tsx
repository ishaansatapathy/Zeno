import Link from "next/link";

import { finale } from "@/content/site";

import { ArrowUpRight, Container, Crosses, Title } from "./frame";

/** The last ask: one statement in a frame, and two doors under it that fill silver on hover. */
export function Finale() {
  return (
    <section aria-labelledby="finale-title" className="pb-28 lg:pb-36">
      <Container>
        <div className="relative border border-line bg-ink/60">
          <Crosses />
          <div className="px-6 py-20 text-center sm:py-28">
            <Title id="finale-title" text={finale.title} className="text-[clamp(2.8rem,6.4vw,6rem)]" />
          </div>
          <div className="grid border-t border-line sm:grid-cols-2">
            {finale.ctas.map((c, i) => (
              <Link
                key={c.label}
                href={c.href}
                prefetch={false}
                className={`fillup group block px-6 py-8 sm:px-10 ${i ? "border-t border-line sm:border-l sm:border-t-0" : ""}`}
              >
                <span className="label block transition-colors group-hover:text-ink/70">{c.tag}</span>
                <span className="mt-5 flex items-center justify-between gap-4">
                  <span className="font-serif text-[clamp(2rem,3.2vw,2.8rem)] italic">{c.label}</span>
                  <ArrowUpRight className="size-5" />
                </span>
                <span className="mt-3 block border-t border-line pt-3 text-[0.88rem] text-mute transition-colors group-hover:border-ink/25 group-hover:text-ink/75">{c.sub}</span>
              </Link>
            ))}
          </div>
        </div>
      </Container>
    </section>
  );
}
