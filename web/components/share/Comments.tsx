"use client";

import Link from "next/link";
import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent, type MutableRefObject } from "react";

import { api } from "@/lib/api/client";

import { EMOJIS, ago, atLabel, marks, mergeComments, refreshFirstPage, refusal, removeComment, type Comment, type PlayerControl } from "./conversation";

type Emoji = (typeof EMOJIS)[number];
const PAGE = 100;
/** How often the conversation is asked for again while somebody looks at it. */
const REFRESH_SECONDS = 20;
const MAX_BODY = 1000;

/**
 * What people say about a shared video: comments and reactions, each of which can be pinned to a moment.
 * Anyone with the link can read; leaving one needs a signed-in account (an attempt without one says so and
 * offers the sign-in). Pressing a moment seeks the video to it. The owner can remove anything; an author,
 * their own. Nothing here is trusted as markup: every word is drawn as text.
 */
export function Comments({
  code,
  token,
  durationSeconds,
  control,
  playing,
}: {
  code: string;
  /** For a link with a password: the token of the viewing that unlocked it. */
  token?: string;
  durationSeconds: number | null;
  control: MutableRefObject<PlayerControl | null>;
  /** Whether a viewing is under way, so there is a moment to pin to. */
  playing: boolean;
}) {
  const [items, setItems] = useState<Comment[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [enabled, setEnabled] = useState(true);
  const [loaded, setLoaded] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [text, setText] = useState("");
  const [pin, setPin] = useState(true);
  const [posting, setPosting] = useState(false);
  const [problem, setProblem] = useState<string>();
  const [signIn, setSignIn] = useState(false);
  const [status, setStatus] = useState("");
  const [, tick] = useState(0);
  const attempt = useRef<{ id: string; text: string; at: number | null } | null>(null);
  /** How many pages are loaded: the first page's cursor is only the cursor while that is the only one. */
  const pages = useRef(1);
  const alive = useRef(true);
  const shown = useRef<Comment[]>([]);
  shown.current = items;

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  const first = useCallback(
    async (quiet: boolean) => {
      const result = await api.share.comments(code, { limit: PAGE, ...(token ? { t: token } : {}) });
      if (!alive.current) return;
      if (!result.ok) {
        // A link with a password shows nothing until it is unlocked; a closed one is the page's business.
        if (!quiet) setLoaded(true);
        return;
      }
      const page = result.data;
      setEnabled(page.comments_enabled);
      setItems((now) => (quiet ? refreshFirstPage(now, page.comments, page.next_cursor !== null && page.next_cursor !== undefined) : mergeComments([], page.comments)));
      // A refresh leaves the cursor of the oldest page loaded alone once there is more than the first.
      if (!quiet) pages.current = 1;
      if (!quiet || pages.current === 1) setCursor(page.next_cursor ?? null);
      setLoaded(true);
    },
    [code, token],
  );

  useEffect(() => {
    setLoaded(false);
    void first(false);
    const timer = setInterval(() => {
      if (!document.hidden) void first(true);
    }, REFRESH_SECONDS * 1000);
    return () => clearInterval(timer);
  }, [first]);

  // "5 minutes ago" stays true while the page is open.
  useEffect(() => {
    const timer = setInterval(() => tick((n) => n + 1), 30_000);
    return () => clearInterval(timer);
  }, []);

  const durationMs = durationSeconds ? durationSeconds * 1000 : (control.current?.durationMs() ?? null);
  const timeline = useMemo(() => marks(items, durationMs), [items, durationMs]);

  async function older() {
    if (!cursor || loadingMore) return;
    setLoadingMore(true);
    const result = await api.share.comments(code, { limit: PAGE, cursor, ...(token ? { t: token } : {}) });
    if (!alive.current) return;
    setLoadingMore(false);
    if (result.ok) {
      pages.current += 1;
      setItems((now) => mergeComments(now, result.data.comments));
      setCursor(result.data.next_cursor ?? null);
    }
  }

  async function leave(body: { kind: "comment"; body: string; at_ms?: number } | { kind: "reaction"; emoji: Emoji; at_ms: number }, again: { id: string }) {
    setProblem(undefined);
    setSignIn(false);
    const result = await api.share.comment(code, { id: again.id, ...body }, token);
    if (!alive.current) return false;
    if (result.ok) {
      setItems((now) => mergeComments(now, [result.data.comment]));
      attempt.current = null;
      setStatus(body.kind === "reaction" ? "Reaction added." : "Comment added.");
      return true;
    }
    if (result.error.status === 401) setSignIn(true);
    else setProblem(refusal(result.error));
    return false;
  }

  async function submit(event: FormEvent) {
    event.preventDefault();
    const body = text.trim();
    if (!body || posting) return;
    if ([...body].length > MAX_BODY) return setProblem(`At most ${MAX_BODY} characters.`);
    const at = pin && control.current ? Math.round(control.current.positionMs()) : null;
    // Trying the same words again after a lost answer is the same comment, not a second one.
    const same = attempt.current && attempt.current.text === body && attempt.current.at === at;
    const id = same && attempt.current ? attempt.current.id : crypto.randomUUID();
    attempt.current = { id, text: body, at };
    setPosting(true);
    const done = await leave({ kind: "comment", body, ...(at !== null ? { at_ms: at } : {}) }, { id });
    if (alive.current) {
      setPosting(false);
      if (done) setText("");
    }
  }

  async function react(emoji: Emoji) {
    const position = control.current?.positionMs();
    if (position === undefined || posting) return;
    setPosting(true);
    await leave({ kind: "reaction", emoji, at_ms: Math.round(position) }, { id: crypto.randomUUID() });
    if (alive.current) setPosting(false);
  }

  async function remove(comment: Comment) {
    const before = shown.current;
    setItems((now) => removeComment(now, comment.id));
    const result = await api.comments.remove(comment.id);
    if (!alive.current) return;
    if (result.ok || result.error.status === 404) return setStatus("Removed.");
    setItems(before);
    setProblem(refusal(result.error));
  }

  function seek(ms: number) {
    control.current?.seek(ms);
  }

  if (!loaded) return null;
  if (!enabled) return null;

  const withMoment = pin && playing;
  const signInHref = `/login?next=${encodeURIComponent(`/s/${code}`)}`;

  return (
    <section aria-labelledby="comments-heading" className="mt-10">
      <div className="flex flex-wrap items-baseline justify-between gap-3">
        <h2 id="comments-heading" className="label">
          Comments{items.length ? ` · ${items.length}${cursor ? "+" : ""}` : ""}
        </h2>
        <div role="group" aria-label="React at this moment" className="flex gap-1">
          {EMOJIS.map((emoji) => (
            <button
              key={emoji}
              type="button"
              onClick={() => void react(emoji)}
              disabled={!playing || posting}
              title={playing ? `React ${emoji} at ${atLabel(control.current?.positionMs() ?? 0)}` : "Play the video to react at a moment"}
              aria-label={`React with ${emoji}`}
              className="h-9 w-9 border border-line text-[1.05rem] transition-colors hover:border-silver disabled:cursor-not-allowed disabled:opacity-40"
            >
              {emoji}
            </button>
          ))}
        </div>
      </div>

      {timeline.length > 0 ? (
        <div role="group" aria-label="Moments people reacted to" className="relative mt-4 h-8 border border-line bg-ink">
          {timeline.map((mark) => (
            <button
              key={mark.atMs}
              type="button"
              onClick={() => seek(mark.atMs)}
              disabled={!playing}
              style={{ left: `${mark.at * 100}%` }}
              title={`${atLabel(mark.atMs)}${mark.count > 1 ? `, ${mark.count} here` : ""}`}
              aria-label={`${mark.count > 1 ? `${mark.count} comments and reactions` : "A comment or reaction"} at ${atLabel(mark.atMs)}`}
              className="absolute top-0 flex h-full -translate-x-1/2 items-center gap-0.5 px-0.5 text-[0.8rem] leading-none hover:bg-panel disabled:cursor-default"
            >
              {mark.emojis[0] ? <span aria-hidden="true">{mark.emojis[0]}</span> : <span aria-hidden="true" className="block h-3 w-px bg-silver" />}
              {mark.count > 1 ? (
                <span aria-hidden="true" className="font-mono text-[0.65rem] text-mute">
                  {mark.count}
                </span>
              ) : null}
            </button>
          ))}
        </div>
      ) : null}

      <form onSubmit={submit} className="mt-5">
        <label htmlFor="comment-text" className="sr-only">
          Add a comment
        </label>
        <textarea
          id="comment-text"
          value={text}
          onChange={(event) => setText(event.target.value)}
          rows={2}
          maxLength={MAX_BODY}
          placeholder="Add a comment"
          className="w-full resize-y border border-line-hi bg-ink px-3 py-2 text-[0.95rem] text-paper placeholder:text-mute focus:border-silver"
        />
        <div className="mt-2 flex flex-wrap items-center justify-between gap-3">
          <label className={`flex items-center gap-2 text-[0.8125rem] ${playing ? "text-mute" : "text-mute/50"}`}>
            <input type="checkbox" checked={pin && playing} disabled={!playing} onChange={(event) => setPin(event.target.checked)} />
            {playing ? `At ${atLabel(control.current?.positionMs() ?? 0)}` : "Play the video to pin a moment"}
          </label>
          <button type="submit" disabled={posting || !text.trim()} className="caps fillup fillup-ink h-10 bg-silver px-5 text-ink disabled:opacity-50">
            {posting ? "Posting…" : withMoment ? "Comment at this moment" : "Comment"}
          </button>
        </div>
        {signIn ? (
          <p role="alert" className="mt-3 text-[0.875rem] text-paper">
            <Link href={signInHref} className="underline decoration-line-hi underline-offset-4 hover:decoration-silver">
              Sign in
            </Link>{" "}
            to leave a comment. Anyone with the link can read them without an account.
          </p>
        ) : null}
        {problem ? (
          <p role="alert" className="mt-3 text-[0.8125rem] text-fault">
            {problem}
          </p>
        ) : null}
      </form>

      <p role="status" aria-live="polite" className="sr-only">
        {status}
      </p>

      {items.length === 0 ? (
        <p className="mt-6 text-[0.9rem] text-mute">No comments yet. Be the first.</p>
      ) : (
        <ul className="mt-6 divide-y divide-line border-y border-line">
          {items.map((comment) => (
            <li key={comment.id} className="flex gap-3 py-4">
              <div className="min-w-0 flex-1">
                <p className="flex flex-wrap items-baseline gap-x-2 text-[0.8125rem] text-mute">
                  <span className="text-paper">{comment.author.name}</span>
                  <span>{ago(comment.created_at)}</span>
                  {comment.at_ms !== null && comment.at_ms !== undefined ? (
                    <button
                      type="button"
                      onClick={() => seek(comment.at_ms ?? 0)}
                      disabled={!playing}
                      aria-label={`Go to ${atLabel(comment.at_ms)}`}
                      className="font-mono text-silver underline decoration-line-hi underline-offset-4 hover:decoration-silver disabled:cursor-default disabled:no-underline"
                    >
                      {atLabel(comment.at_ms)}
                    </button>
                  ) : null}
                </p>
                {comment.kind === "reaction" ? (
                  <p className="mt-1 text-[1.4rem] leading-none" aria-label={`Reacted ${comment.emoji}`}>
                    {comment.emoji}
                  </p>
                ) : (
                  <p className="mt-1 whitespace-pre-wrap break-words text-[0.95rem] leading-relaxed text-paper">{comment.body}</p>
                )}
              </div>
              {comment.can_delete ? (
                <button
                  type="button"
                  onClick={() => void remove(comment)}
                  aria-label={comment.mine ? "Remove your comment" : `Remove the comment by ${comment.author.name}`}
                  className="caps h-8 self-start px-2 text-paper/60 transition-colors hover:text-fault"
                >
                  Remove
                </button>
              ) : null}
            </li>
          ))}
        </ul>
      )}
      {cursor ? (
        <button type="button" onClick={() => void older()} disabled={loadingMore} className="caps mt-4 h-10 border border-line-hi px-5 text-paper/80 hover:text-paper disabled:opacity-60">
          {loadingMore ? "Loading…" : "Older comments"}
        </button>
      ) : null}
    </section>
  );
}
