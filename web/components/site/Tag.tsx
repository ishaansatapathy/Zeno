import { cn } from "@/components/ui/cn";

/** A small pill that names a section: "● How it works". */
export function Tag({ children, className, live = false }: { children: string; className?: string; live?: boolean }) {
  return (
    <p
      className={cn(
        "inline-flex w-fit items-center gap-2 rounded-full border border-white/10 bg-white/[0.03] px-3 py-1 font-mono text-[0.72rem] text-text-muted",
        className,
      )}
    >
      <span className={cn("h-1.5 w-1.5 rounded-full", live ? "bg-success shadow-[0_0_10px_rgb(var(--success))]" : "bg-accent")} />
      {children}
    </p>
  );
}
