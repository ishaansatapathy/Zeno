import Link from "next/link";
import type { ReactNode } from "react";

import type { Title as TitleText } from "@/content/site";

import { Mark } from "../site/Mark";
import { Crosses } from "../site/frame";

/**
 * The frame every account screen sits in: the wordmark, a statement in the serif with its second
 * half in silver, a line of explanation, then the form. Boxes and hairlines, like the rest of the
 * site; nothing here moves.
 */
export function AuthShell({
  title,
  intro,
  children,
  footer,
}: {
  title: TitleText;
  intro?: ReactNode;
  children: ReactNode;
  /** Links to the neighbouring screens, set under the box. */
  footer?: ReactNode;
}) {
  return (
    <main id="main" className="flex min-h-dvh flex-col items-center justify-center px-4 py-20">
      <div className="relative w-full max-w-[26rem] border border-line bg-panel/95 p-7 sm:p-9">
        <Crosses />
        <Link href="/" aria-label="Zeno, home" className="inline-flex text-[0.82rem]">
          <Mark />
        </Link>
        <h1 className="display mt-10 text-[2.5rem] sm:text-[2.9rem]">
          <span className="block">{title[0]}</span>
          <span className="block">
            <em className="metal">{title[1]}</em>
          </span>
        </h1>
        {intro ? <p className="mt-4 text-[0.95rem] leading-relaxed text-mute">{intro}</p> : null}
        <div className="mt-8">{children}</div>
      </div>
      {footer ? <p className="mt-6 max-w-[26rem] text-center text-[0.875rem] text-mute">{footer}</p> : null}
    </main>
  );
}

/** A link in the footer line under a box. */
export function FooterLink({ href, children }: { href: string; children: ReactNode }) {
  return (
    <Link href={href} className="text-paper underline decoration-line-hi underline-offset-4 transition-colors hover:decoration-silver">
      {children}
    </Link>
  );
}
