import Link from "next/link";
import type { ReactNode } from "react";

import type { Title as TitleText } from "@/content/site";

/** A headline in two tones: the claim in white, the follow-through in gray. */
export function Title({ text, as: Tag = "h2", id, className = "", tone = "dark" }: { text: TitleText; as?: "h1" | "h2" | "h3"; id?: string; className?: string; tone?: "dark" | "light" }) {
  return (
    <Tag id={id} className={`display ${className}`}>
      <span className="block">{text[0]}</span>
      <span className={`block ${tone === "dark" ? "text-mute" : "text-bone-mute"}`}>{text[1]}</span>
    </Tag>
  );
}

/** A section's index and name, in small mono capitals, with a short rule before them. */
export function Label({ n, children, className = "" }: { n?: string; children: ReactNode; className?: string }) {
  return (
    <p className={`label flex items-center gap-3 ${className}`}>
      {n ? <span className="font-semibold">{n}</span> : null}
      <span aria-hidden="true" className="h-px w-6 bg-current opacity-40" />
      {children}
    </p>
  );
}

/** The page's width and side margins. */
export function Container({ children, className = "" }: { children: ReactNode; className?: string }) {
  return <div className={`mx-auto w-full max-w-page px-5 sm:px-8 ${className}`}>{children}</div>;
}

/** How every section opens: index and name, the headline on the left, a short note on the right. */
export function SectionHead({ id, n, label, title, aside, compact = false }: { id: string; n: string; label: string; title: TitleText; aside: string; compact?: boolean }) {
  return (
    <div className="grid gap-6 lg:grid-cols-[minmax(0,1fr)_22rem] lg:items-end lg:gap-16">
      <div>
        <Label n={n}>{label}</Label>
        <Title id={id} text={title} className={compact ? "mt-5 text-[clamp(2.2rem,min(4.2vw,7svh),3.8rem)]" : "mt-6 text-[clamp(2.4rem,5vw,4.6rem)]"} />
      </div>
      <p className="max-w-[22rem] text-[0.95rem] leading-relaxed text-mute lg:pb-2">{aside}</p>
    </div>
  );
}

/** A hairline panel with softly rounded corners: where anything interactive lives. */
export function Panel({ children, className = "" }: { children: ReactNode; className?: string }) {
  return <div className={`overflow-hidden rounded-xl border border-line bg-panel/80 ${className}`}>{children}</div>;
}

const btnTone = {
  solid: "btn btn-solid",
  line: "btn btn-line",
  dark: "btn btn-dark",
  "line-dark": "btn btn-line-dark",
} as const;

/** A button that is a link: in-page anchors stay plain, routes go through Next without prefetching. */
export function Button({ href, children, tone = "solid", arrow = true, className = "" }: { href: string; children: string; tone?: keyof typeof btnTone; arrow?: boolean; className?: string }) {
  const inner = (
    <>
      {children}
      {arrow ? <ArrowRight className="size-3.5 transition-transform duration-200 group-hover:translate-x-0.5" /> : null}
    </>
  );
  const cls = `group ${btnTone[tone]} ${className}`;
  return href.startsWith("#") ? (
    <a href={href} className={cls}>
      {inner}
    </a>
  ) : (
    <Link href={href} prefetch={false} className={cls}>
      {inner}
    </Link>
  );
}

export function ArrowRight({ className = "size-3" }: { className?: string }) {
  return (
    <svg viewBox="0 0 12 12" aria-hidden="true" focusable="false" className={className}>
      <path d="M1.5 6h9M7 2.5 10.5 6 7 9.5" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}
