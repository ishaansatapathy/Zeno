// What the comments panel does with the API's answers, as plain functions: no React, no network, so each rule
// can be tested alone.

import type { Schemas } from "@/lib/api/client";

export type Comment = Schemas["CommentResponse"];

/** What the comments need of the player: where it is, how long the video is, and a way to go somewhere in it. */
export interface PlayerControl {
  positionMs(): number;
  durationMs(): number | null;
  seek(ms: number): void;
}

/** The reactions on offer: the same short list the API accepts. */
export const EMOJIS = ["👍", "❤️", "😂", "🔥", "👏", "😮"] as const;

/** How the API sorts a conversation: newest first, the id breaking ties between equal times. */
export function newestFirst(a: Comment, b: Comment): number {
  if (a.created_at !== b.created_at) return a.created_at < b.created_at ? 1 : -1;
  return a.id < b.id ? 1 : -1;
}

/** Adds what was fetched to what is shown. A comment on both appears once, as the newer copy says. */
export function mergeComments(shown: Comment[], fetched: Comment[]): Comment[] {
  const byId = new Map(shown.map((comment) => [comment.id, comment]));
  for (const comment of fetched) byId.set(comment.id, comment);
  return [...byId.values()].sort(newestFirst);
}

/**
 * The first page, fetched again. What it holds replaces what is shown from its newest to its oldest, so a comment
 * removed since is gone; older pages already loaded stay, unless the first page was the whole conversation
 * (`more` false), when nothing older exists.
 */
export function refreshFirstPage(shown: Comment[], first: Comment[], more: boolean): Comment[] {
  const oldest = first[first.length - 1];
  if (!oldest || !more) return mergeComments([], first);
  const older = shown.filter((comment) => newestFirst(comment, oldest) > 0);
  return mergeComments(older, first);
}

export function removeComment(shown: Comment[], id: string): Comment[] {
  return shown.filter((comment) => comment.id !== id);
}

/** Where a comment sits along the video, 0 to 1, or `null` if it is about the whole video (or the length is not known). */
export function position(comment: Pick<Comment, "at_ms">, durationMs: number | null): number | null {
  if (comment.at_ms === null || comment.at_ms === undefined || !durationMs || durationMs <= 0) return null;
  return Math.min(1, Math.max(0, comment.at_ms / durationMs));
}

export interface Mark {
  /** 0 to 1 along the video. */
  at: number;
  /** How many there are here (comments and reactions together). */
  count: number;
  /** The reactions here, most used first (each once). */
  emojis: string[];
  /** The earliest moment of them, in milliseconds: where pressing the mark goes. */
  atMs: number;
}

/**
 * The marks on the timeline: everything with a moment, grouped into `buckets` equal slices of the video so
 * a hundred reactions in the same minute are one mark with a number, not a hundred overlapping dots.
 */
export function marks(comments: Comment[], durationMs: number | null, buckets = 60): Mark[] {
  const grouped = new Map<number, { count: number; atMs: number; emojis: Map<string, number> }>();
  for (const comment of comments) {
    const at = position(comment, durationMs);
    if (at === null || comment.at_ms === null || comment.at_ms === undefined) continue;
    const bucket = Math.min(buckets - 1, Math.floor(at * buckets));
    const slot = grouped.get(bucket) ?? { count: 0, atMs: comment.at_ms, emojis: new Map() };
    slot.count += 1;
    slot.atMs = Math.min(slot.atMs, comment.at_ms);
    if (comment.emoji) slot.emojis.set(comment.emoji, (slot.emojis.get(comment.emoji) ?? 0) + 1);
    grouped.set(bucket, slot);
  }
  return [...grouped.entries()]
    .sort(([a], [b]) => a - b)
    .map(([bucket, slot]) => ({
      at: (bucket + 0.5) / buckets,
      count: slot.count,
      emojis: [...slot.emojis.entries()].sort((a, b) => b[1] - a[1]).map(([emoji]) => emoji),
      atMs: slot.atMs,
    }));
}

/** 83 seconds is "1:23"; an hour and a bit is "1:02:03". */
export function atLabel(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const two = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${two(m)}:${two(s)}` : `${m}:${two(s)}`;
}

/** "just now", "5 minutes ago", "2 hours ago", "3 days ago", then the date. */
export function ago(createdAt: string, now: number = Date.now()): string {
  const seconds = Math.max(0, Math.round((now - new Date(createdAt).getTime()) / 1000));
  const unit = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"} ago`;
  if (seconds < 45) return "just now";
  if (seconds < 3600) return unit(Math.max(1, Math.round(seconds / 60)), "minute");
  if (seconds < 86_400) return unit(Math.round(seconds / 3600), "hour");
  if (seconds < 7 * 86_400) return unit(Math.round(seconds / 86_400), "day");
  return new Intl.DateTimeFormat(undefined, { dateStyle: "medium" }).format(new Date(createdAt));
}

/** What to tell somebody whose attempt to leave a comment was refused, in words for them. */
export function refusal(failure: { status: number; code: string; message: string; retryAfter: number | null }): string {
  switch (failure.code) {
    case "comments_off":
      return "The owner has turned comments off for this video.";
    case "comments_full":
      return "This video has as many comments as it can hold.";
    case "too_many_comments":
      return "You have left as many comments on this video as one person can.";
    case "too_many_requests":
      return failure.retryAfter ? `Slow down a little. Try again in ${failure.retryAfter} second${failure.retryAfter === 1 ? "" : "s"}.` : "Slow down a little, then try again.";
    case "link_unavailable":
      return "This link is no longer available.";
    case "network":
      return failure.message;
    default:
      return failure.status === 422 ? failure.message : "That did not go through. Please try again.";
  }
}
