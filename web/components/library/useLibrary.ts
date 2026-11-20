"use client";

import { useCallback, useEffect, useRef, useState } from "react";

import { api, type ApiFailure } from "@/lib/api/client";

import { addPage, removeRow, replaceRow, type Recording, type Source } from "./rows";

/** How many recordings one request asks for. */
export const PAGE_SIZE = 20;

export type Filter = Source | "all";

export interface Library {
  phase: "loading" | "ready" | "failed";
  rows: Recording[];
  /** Whether there are older recordings than the ones shown. */
  more: boolean;
  loadingMore: boolean;
  failure: ApiFailure | null;
}

const EMPTY: Library = { phase: "loading", rows: [], more: false, loadingMore: false, failure: null };

/**
 * The signed-in person's library for one filter, a page at a time. Changing the filter starts again;
 * an answer that arrives after the person has moved on is thrown away rather than shown.
 */
export function useLibrary(filter: Filter) {
  const [library, setLibrary] = useState<Library>(EMPTY);
  const cursor = useRef<string | null>(null);
  const latest = useRef(0);

  const fetchPage = useCallback(
    async (after: string | null) => {
      const mine = ++latest.current;
      setLibrary((now) => (after ? { ...now, loadingMore: true } : { ...EMPTY }));
      const result = await api.recordings.list({
        limit: PAGE_SIZE,
        ...(after ? { cursor: after } : {}),
        ...(filter === "all" ? {} : { source: filter }),
      });
      if (mine !== latest.current) return;
      if (!result.ok) {
        setLibrary((now) => ({ ...now, phase: after ? "ready" : "failed", loadingMore: false, failure: result.error }));
        return;
      }
      cursor.current = result.data.next_cursor ?? null;
      setLibrary((now) => ({
        phase: "ready",
        rows: addPage(after ? now.rows : [], result.data.recordings),
        more: cursor.current !== null,
        loadingMore: false,
        failure: null,
      }));
    },
    [filter],
  );

  useEffect(() => {
    cursor.current = null;
    void fetchPage(null);
  }, [fetchPage]);

  return {
    library,
    reload: () => fetchPage(null),
    loadMore: () => (cursor.current ? fetchPage(cursor.current) : Promise.resolve()),
    replace: (row: Recording) => setLibrary((now) => ({ ...now, rows: replaceRow(now.rows, row) })),
    remove: (id: string) => setLibrary((now) => ({ ...now, rows: removeRow(now.rows, id) })),
  };
}
