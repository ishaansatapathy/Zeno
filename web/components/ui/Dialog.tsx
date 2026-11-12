"use client";

import { useEffect, useId, useRef, type ReactNode } from "react";

import { cn } from "./cn";

type DialogProps = {
  open: boolean;
  onClose: () => void;
  title: string;
  closeLabel: string;
  children: ReactNode;
  className?: string;
};

/**
 * A modal dialog on the platform's own `<dialog>`: focus moves in and is kept inside, Escape
 * closes it, the page behind is inert, and focus returns to what opened it. Clicking the
 * backdrop closes it too.
 */
export function Dialog({ open, onClose, title, closeLabel, children, className }: DialogProps) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (open && !dialog.open) dialog.showModal();
    if (!open && dialog.open) dialog.close();
  }, [open]);

  return (
    <dialog
      ref={ref}
      aria-labelledby={titleId}
      // Escape, and anything else that closes it from the platform's side, tells the owner.
      onClose={onClose}
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
      className={cn(
        "w-[min(32rem,calc(100vw-2rem))] rounded-card border border-border bg-bg p-0 text-text backdrop:bg-black/50",
        className,
      )}
    >
      <div className="flex flex-col gap-4 p-6">
        <div className="flex items-start justify-between gap-4">
          <h2 id={titleId} className="text-lg font-semibold">
            {title}
          </h2>
          <button
            type="button"
            onClick={onClose}
            aria-label={closeLabel}
            className="-m-2 rounded-control p-2 text-text-muted hover:bg-surface-strong"
          >
            <span aria-hidden="true">✕</span>
          </button>
        </div>
        {children}
      </div>
    </dialog>
  );
}
