"use client";

import { useSyncExternalStore } from "react";

/**
 * Whether a media query matches, kept in sync. The server (and the first paint) assumes it does
 * not, so sections render their simplest form and upgrade after hydration.
 */
export function useMedia(query: string): boolean {
  return useSyncExternalStore(
    (notify) => {
      const list = window.matchMedia(query);
      list.addEventListener("change", notify);
      return () => list.removeEventListener("change", notify);
    },
    () => window.matchMedia(query).matches,
    () => false,
  );
}
