import type { ReactNode } from "react";

/**
 * The one word in a headline that is set apart: the serif's italic, at a slightly larger size so
 * it sits on the same visual line as the grotesk around it. Used through `t.rich(key, rich)`.
 */
export const rich = {
  i: (chunks: ReactNode) => (
    <span className="font-serif text-[1.08em] font-normal italic tracking-[-0.01em]">{chunks}</span>
  ),
};
