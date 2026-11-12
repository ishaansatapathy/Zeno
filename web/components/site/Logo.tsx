import Link from "next/link";

/** The wordmark: a small screen with a play mark, and the name in heavy slanted capitals. */
export function Logo({ className = "" }: { className?: string }) {
  return (
    <Link href="/" aria-label="Zeno, home" className={`group inline-flex items-center gap-2 ${className}`}>
      <svg viewBox="0 0 28 22" aria-hidden="true" focusable="false" className="h-[1.15em] w-auto text-violet-hi">
        <rect x="1.5" y="1.5" width="25" height="19" rx="5" fill="none" stroke="currentColor" strokeWidth="2.6" />
        <path d="M11.4 7.2v7.6l6.6-3.8-6.6-3.8Z" fill="#ffc21a" />
      </svg>
      <span className="shout text-[1.45em] leading-none">Zeno</span>
    </Link>
  );
}
