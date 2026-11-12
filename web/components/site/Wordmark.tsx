/** The mark: a node on the line, an ink ring with a saffron core, and the name in the serif's italic. */
export function Wordmark({ className }: { className?: string }) {
  return (
    <span className={className}>
      <span className="inline-flex items-center gap-2">
        <svg aria-hidden="true" viewBox="0 0 24 24" className="h-[1.05em] w-[1.05em]">
          <path d="M0 12h4.5M19.5 12H24" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
          <circle cx="12" cy="12" r="7" fill="none" stroke="currentColor" strokeWidth="1.8" />
          <circle cx="12" cy="12" r="3.4" fill="rgb(var(--accent))" />
        </svg>
        <span className="font-serif text-[1.55em] italic leading-none tracking-[-0.01em]">Zeno</span>
      </span>
    </span>
  );
}
