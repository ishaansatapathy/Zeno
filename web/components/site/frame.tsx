import Link from "next/link";
import type { ReactNode } from "react";

import type { Title as TitleText } from "@/content/site";

/** A small + at each corner of a box, the registration marks of a drafting sheet. */
export function Crosses({ className = "text-line-hi" }: { className?: string }) {
  const at = ["-left-[5px] -top-[5px]", "-right-[5px] -top-[5px]", "-bottom-[5px] -left-[5px]", "-bottom-[5px] -right-[5px]"];
  return (
    <>
      {at.map((pos) => (
        <svg key={pos} viewBox="0 0 10 10" aria-hidden="true" focusable="false" className={`pointer-events-none absolute size-[10px] ${pos} ${className}`}>
          <path d="M5 0v10M0 5h10" stroke="currentColor" strokeWidth="1" />
        </svg>
      ))}
    </>
  );
}

/** A statement in the serif; its second half in the italic, polished silver. */
export function Title({ text, as: Tag = "h2", id, className = "" }: { text: TitleText; as?: "h1" | "h2" | "h3"; id?: string; className?: string }) {
  return (
    <Tag id={id} className={`display ${className}`}>
      <span className="block">{text[0]}</span>
      <span className="block">
        <em className="metal">{text[1]}</em>
      </span>
    </Tag>
  );
}

/** A small square and a name in mono capitals: how every section introduces itself. */
export function Label({ n, children, className = "" }: { n?: string; children: ReactNode; className?: string }) {
  return (
    <p className={`label flex items-center gap-2.5 ${className}`}>
      <span aria-hidden="true" className="size-1.5 bg-current" />
      {n ? <span>{n} /</span> : null}
      {children}
    </p>
  );
}

/** The page's width and side margins. */
export function Container({ children, className = "" }: { children: ReactNode; className?: string }) {
  return <div className={`mx-auto w-full max-w-page px-4 sm:px-6 lg:px-10 ${className}`}>{children}</div>;
}

/** How a section opens: its label, the statement on the left, a short note at the right. */
export function SectionHead({ id, n, label, title, aside, compact = false }: { id: string; n: string; label: string; title: TitleText; aside: string; compact?: boolean }) {
  return (
    <div className="grid gap-7 lg:grid-cols-[minmax(0,1fr)_minmax(0,24rem)] lg:items-end lg:gap-16">
      <div>
        <Label n={n}>{label}</Label>
        <Title id={id} text={title} className={compact ? "mt-5 text-[clamp(2.6rem,min(5vw,8.5svh),4.8rem)]" : "mt-6 text-[clamp(2.8rem,6vw,5.6rem)]"} />
      </div>
      <p className="max-w-[24rem] text-[0.95rem] leading-relaxed text-mute lg:pb-3">{aside}</p>
    </div>
  );
}

const tones = {
  silver: "fillup fillup-ink bg-silver text-ink",
  line: "fillup border border-line-hi text-paper",
  ink: "fillup fillup-white bg-ink text-silver-hi",
  "line-ink": "fillup fillup-ink border border-ink/70 text-ink",
} as const;

/** A rectangular button in tracked capitals that fills from the bottom on hover. */
export function Button({ href, children, tone = "silver", out = false, className = "" }: { href: string; children: string; tone?: keyof typeof tones; out?: boolean; className?: string }) {
  const inner = (
    <>
      {children}
      {out ? <ArrowUpRight /> : <ArrowRight />}
    </>
  );
  const cls = `caps inline-flex h-12 items-center justify-center gap-5 px-6 ${tones[tone]} ${className}`;
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

export function ArrowUpRight({ className = "size-3" }: { className?: string }) {
  return (
    <svg viewBox="0 0 12 12" aria-hidden="true" focusable="false" className={className}>
      <path d="M3 9 9 3M4 3h5v5" fill="none" stroke="currentColor" strokeWidth="1.4" />
    </svg>
  );
}

export function ArrowRight({ className = "size-3" }: { className?: string }) {
  return (
    <svg viewBox="0 0 12 12" aria-hidden="true" focusable="false" className={className}>
      <path d="M1.5 6h9M7 2.5 10.5 6 7 9.5" fill="none" stroke="currentColor" strokeWidth="1.4" />
    </svg>
  );
}
