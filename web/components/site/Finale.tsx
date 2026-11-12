import Link from "next/link";

import { finale } from "@/content/site";

import { ArrowUpRight, Crosses, Title } from "./frame";

/** The last ask: one statement in a frame, and two doors under it. */
export function Finale() {
  return (
    <section aria-labelledby="finale-title" className="mx-auto max-w-page px-4 pb-28 sm:px-6 lg:px-10 lg:pb-36">
      <div className="relative border border-line">
        <Crosses />
        <div className="px-6 py-20 text-center sm:py-28">
          <Title id="finale-title" text={finale.title} className="text-[clamp(2.6rem,5.8vw,5.2rem)]" />
        </div>
        <div className="grid border-t border-line sm:grid-cols-2">
          {finale.ctas.map((c, i) => (
            <Link
              key={c.label}
              href={c.href}
              prefetch={false}
              className={`fillup group block px-6 py-8 sm:px-10 ${i ? "border-t border-line sm:border-l sm:border-t-0" : ""}`}
            >
              <span className="label block text-signal transition-colors group-hover:text-ink">[ {c.tag} ]</span>
              <span className="mt-5 flex items-center justify-between gap-4">
                <span className="display text-[clamp(1.9rem,3vw,2.6rem)] italic">{c.label}</span>
                <ArrowUpRight className="size-5" />
              </span>
              <span className="mt-3 block border-t border-line pt-3 text-[0.85rem] text-mute transition-colors group-hover:border-ink/30 group-hover:text-ink/75">{c.sub}</span>
            </Link>
          ))}
        </div>
      </div>
    </section>
  );
}
