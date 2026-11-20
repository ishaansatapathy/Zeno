/** The wordmark: a silver tile with a play mark cut into it, and the name in the serif. */
export function Mark({ className = "" }: { className?: string }) {
  return (
    <span className={`inline-flex items-center gap-2.5 ${className}`}>
      <svg viewBox="0 0 20 20" aria-hidden="true" focusable="false" className="size-[1.2em]">
        <rect width="20" height="20" fill="#e4e7eb" />
        <path d="M7.5 5.5v9l7-4.5-7-4.5Z" fill="#000000" />
      </svg>
      <span className="font-serif text-[1.45em] leading-none tracking-[-0.01em]">Zeno</span>
    </span>
  );
}
