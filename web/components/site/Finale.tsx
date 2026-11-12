import Link from "next/link";

import { finale } from "@/content/site";

import { ArrowRight, Container, Title } from "./frame";

/** The last ask: one statement, and two ways in under it. */
export function Finale() {
  return (
    <section aria-labelledby="finale-title" className="border-t border-line py-28 lg:py-36">
      <Container>
        <div className="relative overflow-hidden rounded-2xl border border-line bg-panel/80">
          <div aria-hidden="true" className="pointer-events-none absolute inset-0 bg-[radial-gradient(ellipse_55%_70%_at_50%_0%,rgba(180,194,214,0.08),transparent_70%)]" />
          <div className="relative px-6 py-20 text-center sm:py-24">
            <Title id="finale-title" text={finale.title} className="text-[clamp(2.4rem,5.4vw,4.8rem)]" />
          </div>
          <div className="relative grid border-t border-line sm:grid-cols-2">
            {finale.ctas.map((c, i) => (
              <Link
                key={c.label}
                href={c.href}
                prefetch={false}
                className={`group block px-6 py-7 transition-colors duration-300 hover:bg-raise sm:px-9 ${i ? "border-t border-line sm:border-l sm:border-t-0" : ""}`}
              >
                <span className="label block">{c.tag}</span>
                <span className="mt-3 flex items-center justify-between gap-4 text-[1.35rem] font-medium tracking-[-0.02em]">
                  {c.label}
                  <ArrowRight className="size-4 text-mute transition-[transform,color] duration-300 group-hover:translate-x-1 group-hover:text-paper" />
                </span>
                <span className="mt-1 block text-[0.88rem] text-mute">{c.sub}</span>
              </Link>
            ))}
          </div>
        </div>
      </Container>
    </section>
  );
}
