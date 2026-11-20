// The library's list, as plain functions: no React, no network, so each rule can be tested alone.

import type { Schemas } from "@/lib/api/client";

export type Recording = Schemas["RecordingResponse"];
export type Link = Schemas["LinkResponse"];
export type Source = NonNullable<Schemas["CreateRecordingRequest"]["source"]>;

/** The order the API sorts the library in: newest first, the id breaking ties between equal times. */
export function newestFirst(a: Recording, b: Recording): number {
  if (a.created_at !== b.created_at) return a.created_at < b.created_at ? 1 : -1;
  return a.id < b.id ? 1 : -1;
}

/**
 * Adds a page to the rows already shown. A row that is on both (a recording that moved while
 * someone scrolled, or a page fetched twice) appears once, as the newer copy says.
 */
export function addPage(rows: Recording[], page: Recording[]): Recording[] {
  const byId = new Map(rows.map((row) => [row.id, row]));
  for (const row of page) byId.set(row.id, row);
  return [...byId.values()].sort(newestFirst);
}

/** Puts a fresh copy of a recording where the old one was. A recording that is not listed stays unlisted. */
export function replaceRow(rows: Recording[], row: Recording): Recording[] {
  return rows.map((existing) => (existing.id === row.id ? row : existing));
}

export function removeRow(rows: Recording[], id: string): Recording[] {
  return rows.filter((row) => row.id !== id);
}

/**
 * The link to hand out for a recording: one that works, with no password and no end date. Anything
 * with conditions is a link somebody made on purpose for somebody in particular, and is not what
 * "copy link" should silently give to someone else.
 */
export function plainLink(links: Link[]): Link | undefined {
  return links.find((link) => link.state === "active" && !link.has_password && !link.expires_at);
}

export const SOURCES: { value: Source; label: string }[] = [
  { value: "upload", label: "Uploads" },
  { value: "extension", label: "Extension" },
  { value: "studio", label: "Studio" },
  { value: "stream", label: "Streams" },
  { value: "meeting", label: "Meetings" },
];

const SOURCE_NAMES: Record<string, string> = { upload: "Upload", extension: "Extension", studio: "Studio", stream: "Stream", meeting: "Meeting" };
const STATUS_NAMES: Record<string, string> = { capturing: "Capturing", uploading: "Uploading", processing: "Processing", ready: "Ready", failed: "Failed" };
const VISIBILITY_NAMES: Record<string, string> = { private: "Private", link: "Anyone with the link", public: "Public" };

/** A word for what the API said, or the API's own word if this app has not heard of it. */
export const sourceName = (value: string) => SOURCE_NAMES[value] ?? value;
export const statusName = (value: string) => STATUS_NAMES[value] ?? value;
export const visibilityName = (value: string) => VISIBILITY_NAMES[value] ?? value;

/** 83 seconds is "1:23"; an hour and a bit is "1:02:03". */
export function clock(seconds: number): string {
  const total = Math.max(0, Math.round(seconds));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const two = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${two(m)}:${two(s)}` : `${m}:${two(s)}`;
}
