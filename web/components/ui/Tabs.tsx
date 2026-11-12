"use client";

import { useId, useRef, useState, type KeyboardEvent, type ReactNode } from "react";

import { cn } from "./cn";

export type Tab = { id: string; label: string; content: ReactNode };

/**
 * Tabs with the keyboard behaviour screen-reader users expect: one tab stop for the whole list,
 * arrows to move between tabs, Home and End to jump, and the panel is a labelled region.
 */
export function Tabs({ tabs, label, initial }: { tabs: Tab[]; label: string; initial?: string }) {
  const base = useId();
  const [selected, setSelected] = useState(initial && tabs.some((t) => t.id === initial) ? initial : tabs[0]?.id);
  const buttons = useRef<Record<string, HTMLButtonElement | null>>({});

  const move = (to: number) => {
    const tab = tabs[(to + tabs.length) % tabs.length];
    setSelected(tab.id);
    buttons.current[tab.id]?.focus();
  };

  const onKeyDown = (event: KeyboardEvent, index: number) => {
    const target =
      event.key === "ArrowRight"
        ? index + 1
        : event.key === "ArrowLeft"
          ? index - 1
          : event.key === "Home"
            ? 0
            : event.key === "End"
              ? tabs.length - 1
              : null;
    if (target === null) return;
    event.preventDefault();
    move(target);
  };

  return (
    <div>
      <div role="tablist" aria-label={label} className="flex gap-1 border-b border-border">
        {tabs.map((tab, index) => {
          const active = tab.id === selected;
          return (
            <button
              key={tab.id}
              ref={(node) => {
                buttons.current[tab.id] = node;
              }}
              role="tab"
              id={`${base}-tab-${tab.id}`}
              aria-selected={active}
              aria-controls={`${base}-panel-${tab.id}`}
              tabIndex={active ? 0 : -1}
              type="button"
              onClick={() => setSelected(tab.id)}
              onKeyDown={(event) => onKeyDown(event, index)}
              className={cn(
                "-mb-px min-h-11 border-b-2 px-4 font-medium",
                active ? "border-primary text-primary" : "border-transparent text-text-muted hover:text-text",
              )}
            >
              {tab.label}
            </button>
          );
        })}
      </div>
      {tabs.map((tab) => (
        <div
          key={tab.id}
          role="tabpanel"
          id={`${base}-panel-${tab.id}`}
          aria-labelledby={`${base}-tab-${tab.id}`}
          hidden={tab.id !== selected}
          tabIndex={0}
          className="pt-4"
        >
          {tab.id === selected && tab.content}
        </div>
      ))}
    </div>
  );
}
