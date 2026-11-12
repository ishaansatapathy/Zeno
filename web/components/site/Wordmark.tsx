/** The mark: a silver tile with a forward chevron (the next lesson), and the name. */
export function Wordmark({ className }: { className?: string }) {
  return (
    <span className={className}>
      <span className="inline-flex items-center gap-2.5">
        <span aria-hidden="true" className="plate grid h-7 w-7 place-items-center rounded-[0.55rem]">
          <svg viewBox="0 0 16 16" className="h-3.5 w-3.5">
            <path d="M5 3.5 9.5 8 5 12.5" fill="none" stroke="#050505" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        </span>
        <span className="text-[1.1rem] font-semibold tracking-[-0.03em]">Zeno</span>
      </span>
    </span>
  );
}
