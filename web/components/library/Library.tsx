"use client";

import { useEffect, useRef, useState } from "react";

import { Notice } from "@/components/auth/controls";
import { describeFailure } from "@/components/auth/messages";
import { useSession } from "@/lib/session";

import { RecordingRow } from "./RecordingRow";
import { SOURCES } from "./rows";
import { useLibrary, type Filter } from "./useLibrary";

const FILTERS: { value: Filter; label: string }[] = [{ value: "all", label: "All" }, ...SOURCES];

/** What a signed-in person lands on: their recordings, newest first, and what can be done to each. */
export function Library() {
  const { session } = useSession();
  const [filter, setFilter] = useState<Filter>("all");
  const { library, reload, loadMore, replace, remove } = useLibrary(filter);
  const [message, setMessage] = useState("");
  const clearTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const list = useRef<HTMLUListElement>(null);

  useEffect(() => () => void (clearTimer.current && clearTimeout(clearTimer.current)), []);

  function notify(text: string) {
    setMessage(text);
    if (clearTimer.current) clearTimeout(clearTimer.current);
    clearTimer.current = setTimeout(() => setMessage(""), 5000);
  }

  if (session.status !== "signed-in") return null;
  const { rows, phase, failure } = library;
  // A person whose account keeps no library (a student) is told so, not shown an error.
  const noLibrary = failure?.status === 403 && failure.code === "forbidden";

  return (
    <>
      <p className="label">Signed in as {session.user.email}</p>
      <h1 className="display mt-6 text-[clamp(2.4rem,6vw,4.5rem)]">
        <span className="block">Your</span>
        <span className="block">
          <em className="metal">library.</em>
        </span>
      </h1>

      <div className="mt-10 flex flex-wrap items-center justify-between gap-4">
        <div role="group" aria-label="Show recordings made by" className="flex flex-wrap gap-px border border-line-hi bg-line-hi">
          {FILTERS.map((option) => (
            <button
              key={option.value}
              type="button"
              aria-pressed={filter === option.value}
              onClick={() => setFilter(option.value)}
              className={`caps h-10 px-4 transition-colors ${filter === option.value ? "bg-silver text-ink" : "bg-ink text-paper/80 hover:bg-raise hover:text-paper"}`}
            >
              {option.label}
            </button>
          ))}
        </div>
        <p role="status" aria-live="polite" className="label min-h-5 text-paper">
          {message}
        </p>
      </div>

      <div className="mt-6 border-t border-line">
        {phase === "loading" ? (
          <div role="status" className="py-6">
            <span className="sr-only">Loading your library</span>
            {[0, 1, 2].map((n) => (
              <div key={n} aria-hidden="true" className="border-b border-line py-5">
                <div className="h-5 w-2/5 bg-raise" />
                <div className="mt-3 h-3 w-1/4 bg-raise" />
              </div>
            ))}
          </div>
        ) : null}

        {phase === "failed" && failure ? (
          noLibrary ? (
            <p className="py-10 text-[1rem] leading-relaxed text-mute">Your account is for watching, so it has no library. Create a creator account to record and share.</p>
          ) : (
            <div className="py-8">
              <Notice tone="error">{describeFailure(failure)}</Notice>
              <button type="button" onClick={() => void reload()} className="caps fillup fillup-ink h-12 bg-silver px-8 text-ink">
                Try again
              </button>
            </div>
          )
        ) : null}

        {phase === "ready" && rows.length === 0 ? (
          <p className="py-10 text-[1rem] leading-relaxed text-mute">
            {filter === "all" ? "Nothing here yet. Recordings you make will appear here." : "No recordings made this way."}
          </p>
        ) : null}

        {rows.length > 0 ? (
          <ul ref={list} tabIndex={-1} aria-label="Your recordings" className="outline-none">
            {rows.map((row) => (
              <RecordingRow
                key={row.id}
                recording={row}
                onChange={replace}
                onRemove={(id) => {
                  remove(id);
                  list.current?.focus();
                }}
                notify={notify}
              />
            ))}
          </ul>
        ) : null}

        {phase === "ready" && failure ? (
          <div className="pt-6">
            <Notice tone="error">{describeFailure(failure)}</Notice>
          </div>
        ) : null}

        {library.more ? (
          <div className="pt-8">
            <button
              type="button"
              onClick={() => void loadMore()}
              disabled={library.loadingMore}
              className="caps fillup h-12 border border-line-hi px-8 text-paper disabled:cursor-wait disabled:opacity-60"
            >
              {library.loadingMore ? "Loading…" : "Load more"}
            </button>
          </div>
        ) : null}
      </div>
    </>
  );
}
