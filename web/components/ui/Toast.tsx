"use client";

import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";

import { cn } from "./cn";

type Tone = "info" | "success" | "error";
type ToastItem = { id: number; tone: Tone; message: string };

const ToastContext = createContext<((message: string, tone?: Tone) => void) | null>(null);

/** Show a short message. Errors are announced at once; the rest wait for a pause in speech. */
export function useToast() {
  const push = useContext(ToastContext);
  if (!push) throw new Error("useToast needs a <ToastProvider> above it");
  return push;
}

const SHOWN_MS = 6000;
const MAX_SHOWN = 4;

export function ToastProvider({ children, dismissLabel }: { children: ReactNode; dismissLabel: string }) {
  const [items, setItems] = useState<ToastItem[]>([]);
  const next = useRef(1);
  const timers = useRef(new Map<number, ReturnType<typeof setTimeout>>());

  const dismiss = useCallback((id: number) => {
    clearTimeout(timers.current.get(id));
    timers.current.delete(id);
    setItems((all) => all.filter((item) => item.id !== id));
  }, []);

  const push = useCallback(
    (message: string, tone: Tone = "info") => {
      const id = next.current++;
      setItems((all) => [...all.slice(-(MAX_SHOWN - 1)), { id, tone, message }]);
      timers.current.set(
        id,
        setTimeout(() => dismiss(id), SHOWN_MS),
      );
    },
    [dismiss],
  );

  useEffect(() => {
    const live = timers.current;
    return () => live.forEach((timer) => clearTimeout(timer));
  }, []);

  return (
    <ToastContext.Provider value={push}>
      {children}
      <div className="pointer-events-none fixed inset-x-0 bottom-4 z-50 flex flex-col items-center gap-2 px-4">
        {items.map((item) => (
          <div
            key={item.id}
            role={item.tone === "error" ? "alert" : "status"}
            className={cn(
              "pointer-events-auto flex w-full max-w-md items-start gap-3 rounded-control border bg-bg p-3 shadow-lg",
              item.tone === "error" && "border-danger",
              item.tone === "success" && "border-success",
              item.tone === "info" && "border-border-strong",
            )}
          >
            <p className="flex-1 text-sm">{item.message}</p>
            <button
              type="button"
              onClick={() => dismiss(item.id)}
              aria-label={dismissLabel}
              className="-m-1 rounded p-1 text-text-muted hover:bg-surface-strong"
            >
              <span aria-hidden="true">✕</span>
            </button>
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}
