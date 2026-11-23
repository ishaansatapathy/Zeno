"use client";

import { useEffect, useRef, useState } from "react";

import { describeFailure } from "@/components/auth/messages";
import { api } from "@/lib/api/client";
import { limits } from "@/lib/api/limits";

import { clock, plainLink, sourceName, statusName, visibilityName, type Recording } from "./rows";

type Mode = "view" | "rename" | "delete";
type Copy = { phase: "idle" } | { phase: "working" } | { phase: "copied" } | { phase: "manual"; url: string } | { phase: "failed"; message: string };

const action = "caps h-9 px-3 text-paper/80 transition-colors hover:text-paper disabled:cursor-wait disabled:opacity-60";
const date = new Intl.DateTimeFormat(undefined, { dateStyle: "medium" });

function Chip({ children, tone = "plain" }: { children: React.ReactNode; tone?: "plain" | "good" | "bad" }) {
  const colours = tone === "good" ? "border-silver text-paper" : tone === "bad" ? "border-fault/60 text-fault" : "border-line-hi text-mute";
  return <span className={`label inline-block border px-2 py-1 ${colours}`}>{children}</span>;
}

/**
 * One recording in the library: what it is, and what can be done to it. Renaming, deleting and
 * copying its link all happen in the row. Each says what it did through `notify`, which a live
 * region announces, and hands the new copy of the recording to `onChange`.
 */
export function RecordingRow({
  recording,
  onChange,
  onRemove,
  notify,
}: {
  recording: Recording;
  onChange: (recording: Recording) => void;
  onRemove: (id: string) => void;
  notify: (message: string) => void;
}) {
  const [mode, setMode] = useState<Mode>("view");
  const [title, setTitle] = useState(recording.title);
  const [problem, setProblem] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [copy, setCopy] = useState<Copy>({ phase: "idle" });
  const renameButton = useRef<HTMLButtonElement>(null);
  const keepButton = useRef<HTMLButtonElement>(null);
  const titleInput = useRef<HTMLInputElement>(null);
  const manual = useRef<HTMLInputElement>(null);
  const copiedTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => () => void (copiedTimer.current && clearTimeout(copiedTimer.current)), []);
  useEffect(() => {
    if (mode === "rename") titleInput.current?.select();
    if (mode === "delete") keepButton.current?.focus();
  }, [mode]);
  useEffect(() => {
    if (copy.phase === "manual") manual.current?.select();
  }, [copy]);

  function leave() {
    setMode("view");
    setProblem(undefined);
    requestAnimationFrame(() => renameButton.current?.focus());
  }

  function startRename() {
    setTitle(recording.title);
    setProblem(undefined);
    setMode("rename");
  }

  async function saveTitle() {
    if (busy) return;
    const next = title.trim();
    if (next === recording.title) return leave();
    if (!next) return setProblem("Give it a title.");
    if ([...next].length > limits.titleMax) return setProblem(`A title can be at most ${limits.titleMax} characters.`);

    setBusy(true);
    const result = await api.recordings.rename(recording.id, recording.version, next);
    if (result.ok) {
      setBusy(false);
      onChange(result.data.recording);
      notify(`Renamed to ${result.data.recording.title}.`);
      return leave();
    }
    if (result.error.code === "version_conflict") {
      // Someone (another tab, another device) changed it first. Show what it is now, not what we guessed.
      const fresh = await api.recordings.get(recording.id);
      setBusy(false);
      if (fresh.ok) onChange(fresh.data.recording);
      setProblem(undefined);
      notify("This recording changed elsewhere, so it was reloaded. Rename it again if you still want to.");
      return leave();
    }
    setBusy(false);
    setProblem(result.error.fields.title ?? describeFailure(result.error));
    titleInput.current?.focus();
  }

  /** Opens or closes the conversation under this recording's link. What was said is kept either way. */
  async function toggleComments() {
    if (busy) return;
    setBusy(true);
    const next = !recording.comments_enabled;
    const result = await api.recordings.setComments(recording.id, recording.version, next);
    if (result.ok) {
      setBusy(false);
      onChange(result.data.recording);
      return notify(next ? "Comments are on." : "Comments are off. What was said is kept, and can be turned back on.");
    }
    if (result.error.code === "version_conflict") {
      const fresh = await api.recordings.get(recording.id);
      setBusy(false);
      if (fresh.ok) onChange(fresh.data.recording);
      return notify("This recording changed elsewhere, so it was reloaded. Try again if you still want to.");
    }
    setBusy(false);
    notify(describeFailure(result.error));
  }

  async function destroy() {
    if (busy) return;
    setBusy(true);
    const result = await api.recordings.remove(recording.id);
    // Gone already (deleted in another tab) is the outcome that was asked for.
    if (result.ok || result.error.status === 404) {
      onRemove(recording.id);
      notify(`Deleted ${recording.title}.`);
      return;
    }
    setBusy(false);
    setProblem(describeFailure(result.error));
  }

  async function copyLink() {
    if (copy.phase === "working") return;
    setCopy({ phase: "working" });
    const fail = (message: string) => setCopy({ phase: "failed", message });

    const listed = await api.links.list(recording.id);
    if (!listed.ok) return fail(describeFailure(listed.error));
    let link = plainLink(listed.data.links);
    if (!link) {
      const made = await api.links.create(recording.id, {});
      if (!made.ok) return fail(describeFailure(made.error));
      link = made.data.link;
      // Making the first link switches a private recording to "link", which is an edit: take the
      // new version, or the next rename would be refused as out of date.
      const fresh = await api.recordings.get(recording.id);
      if (fresh.ok) onChange(fresh.data.recording);
    }
    try {
      await navigator.clipboard.writeText(link.url);
    } catch {
      // Some browsers and settings refuse; the person can still take it from the box.
      return setCopy({ phase: "manual", url: link.url });
    }
    setCopy({ phase: "copied" });
    notify("Link copied.");
    copiedTimer.current = setTimeout(() => setCopy({ phase: "idle" }), 2500);
  }

  const inFlight = busy || copy.phase === "working";
  const ready = recording.status === "ready";

  return (
    <li className="border-b border-line py-5" aria-busy={inFlight}>
      <div className="flex flex-col gap-4 md:flex-row md:items-start md:justify-between">
        <div className="min-w-0 flex-1">
          {mode === "rename" ? (
            <form
              onSubmit={(event) => {
                event.preventDefault();
                void saveTitle();
              }}
              className="flex flex-col gap-2 sm:flex-row sm:items-start"
            >
              <div className="min-w-0 flex-1">
                <label htmlFor={`title-${recording.id}`} className="sr-only">
                  Title
                </label>
                <input
                  id={`title-${recording.id}`}
                  ref={titleInput}
                  value={title}
                  onChange={(event) => setTitle(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Escape") leave();
                  }}
                  autoFocus
                  autoComplete="off"
                  aria-invalid={problem ? true : undefined}
                  aria-describedby={problem ? `title-error-${recording.id}` : undefined}
                  className={`h-11 w-full border bg-ink px-3 text-[1rem] text-paper ${problem ? "border-fault" : "border-line-hi focus:border-silver"}`}
                />
                {problem ? (
                  <p id={`title-error-${recording.id}`} className="mt-2 text-[0.8125rem] text-fault">
                    {problem}
                  </p>
                ) : null}
              </div>
              <div className="flex gap-2">
                <button type="submit" disabled={busy} className="caps fillup fillup-ink h-11 bg-silver px-4 text-ink disabled:opacity-60">
                  {busy ? "Saving…" : "Save"}
                </button>
                <button type="button" onClick={leave} className="caps h-11 border border-line-hi px-4 text-paper/80 hover:text-paper">
                  Cancel
                </button>
              </div>
            </form>
          ) : (
            <h2 className="break-words text-[1.05rem] font-medium leading-snug text-paper">{recording.title}</h2>
          )}
          <p className="label mt-2">
            {sourceName(recording.source)} · {date.format(new Date(recording.created_at))}
            {ready && recording.duration_seconds ? ` · ${clock(recording.duration_seconds)}` : ""}
          </p>
          <div className="mt-3 flex flex-wrap items-center gap-2">
            <Chip tone={ready ? "good" : recording.status === "failed" ? "bad" : "plain"}>{statusName(recording.status)}</Chip>
            <Chip>{visibilityName(recording.visibility)}</Chip>
            <Chip>
              {recording.view_count.toLocaleString()} {recording.view_count === 1 ? "view" : "views"}
            </Chip>
          </div>
        </div>

        {mode === "delete" ? (
          <div role="group" aria-label={`Delete ${recording.title}?`} className="md:max-w-xs md:text-right">
            <p className="text-[0.875rem] leading-relaxed text-paper">
              Delete this recording? Its video goes too, and every link to it stops working.
            </p>
            <div className="mt-3 flex gap-2 md:justify-end">
              <button type="button" ref={keepButton} onClick={leave} disabled={busy} className="caps h-9 border border-line-hi px-4 text-paper">
                Keep it
              </button>
              <button type="button" onClick={() => void destroy()} disabled={busy} className="caps h-9 border border-fault/60 px-4 text-fault disabled:opacity-60">
                {busy ? "Deleting…" : "Delete"}
              </button>
            </div>
            {problem ? (
              <p role="alert" className="mt-2 text-[0.8125rem] text-fault">
                {problem}
              </p>
            ) : null}
          </div>
        ) : (
          <div className="flex flex-wrap items-center gap-x-1 md:justify-end">
            <button type="button" onClick={() => void copyLink()} disabled={inFlight} aria-label={`Copy link for ${recording.title}`} className={action}>
              {copy.phase === "working" ? "Copying…" : copy.phase === "copied" ? "Copied" : "Copy link"}
            </button>
            <button
              type="button"
              onClick={() => void toggleComments()}
              disabled={inFlight}
              aria-pressed={recording.comments_enabled}
              aria-label={`${recording.comments_enabled ? "Comments on" : "Comments off"} for ${recording.title}`}
              className={action}
            >
              {recording.comments_enabled ? "Comments on" : "Comments off"}
            </button>
            <button type="button" ref={renameButton} onClick={startRename} disabled={inFlight || mode === "rename"} aria-label={`Rename ${recording.title}`} className={action}>
              Rename
            </button>
            <button
              type="button"
              onClick={() => {
                setProblem(undefined);
                setMode("delete");
              }}
              disabled={inFlight}
              aria-label={`Delete ${recording.title}`}
              className={`${action} hover:text-fault`}
            >
              Delete
            </button>
          </div>
        )}
      </div>

      {copy.phase === "manual" ? (
        <div className="mt-3">
          <label htmlFor={`manual-${recording.id}`} className="label mb-2 block">
            Copy this link
          </label>
          <input id={`manual-${recording.id}`} ref={manual} readOnly value={copy.url} className="h-11 w-full border border-line-hi bg-ink px-3 font-mono text-[0.85rem] text-paper" />
        </div>
      ) : null}
      {copy.phase === "failed" ? (
        <p role="alert" className="mt-3 text-[0.8125rem] text-fault">
          {copy.message}
        </p>
      ) : null}
    </li>
  );
}
