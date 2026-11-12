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

/** A title whose second half is set in the serif's italic, in signal orange. */
export function Title({ text, as: Tag = "h2", id, className = "" }: { text: TitleText; as?: "h1" | "h2" | "h3"; id?: string; className?: string }) {
  return (
    <Tag id={id} className={`display ${className}`}>
      <span className="block">{text[0]}</span>
      <em className="block italic text-signal">{text[1]}</em>
    </Tag>
  );
}

/** A label in brackets, the way every section introduces itself. */
export function Label({ children, className = "" }: { children: ReactNode; className?: string }) {
  return <p className={`label ${className}`}>[ {children} ]</p>;
}

/** The standard opening of a section: label, title on the left, a short note on the right. */
export function SectionHead({ id, label, title, aside }: { id: string; label: string; title: TitleText; aside: string }) {
  return (
    <div className="grid gap-8 lg:grid-cols-[minmax(0,1fr)_minmax(0,24rem)] lg:items-end lg:gap-16">
      <div>
        <Label>{label}</Label>
        <Title id={id} text={title} className="mt-6 text-[clamp(2.6rem,5.4vw,5rem)]" />
      </div>
      <p className="max-w-[24rem] text-[0.95rem] leading-relaxed text-mute lg:pb-3">{aside}</p>
    </div>
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
