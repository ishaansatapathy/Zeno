/** "01 — HOW IT WORKS": a mono label with a small ink node in front, like a stop on the line. */
export function Label({ index, children }: { index?: string; children: string }) {
  return (
    <p className="label flex items-center gap-3">
      <span aria-hidden="true" className="inline-block h-2 w-2 rounded-full border border-text bg-accent" />
      {index && <span aria-hidden="true">{index}</span>}
      {index && <span aria-hidden="true" className="h-px w-6 bg-border-strong" />}
      <span>{children}</span>
    </p>
  );
}
