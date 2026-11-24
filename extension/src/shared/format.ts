import type { Track } from "./messages";

/** 83 seconds is "1:23"; an hour and a bit is "1:02:03". */
export function clock(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  const two = (n: number) => String(n).padStart(2, "0");
  return hours > 0 ? `${hours}:${two(minutes)}:${two(seconds)}` : `${minutes}:${two(seconds)}`;
}

/** 1536 is "1.5 KB"; one decimal, and none for plain bytes. */
export function bytesText(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(1)} ${units[unit]}`;
}

const two = (n: number) => String(n).padStart(2, "0");

/**
 * The name a recording is saved under: "Zeno 2026-11-21 14.05.09 screen.webm". Sorted by name is sorted by
 * time, the track is in the name so the files of one recording sit together, and nothing in it is a
 * character a file system refuses.
 */
export function fileName(track: Track, startedAt: number): string {
  const d = new Date(startedAt);
  const date = `${d.getFullYear()}-${two(d.getMonth() + 1)}-${two(d.getDate())}`;
  const time = `${two(d.getHours())}.${two(d.getMinutes())}.${two(d.getSeconds())}`;
  return `Zeno ${date} ${time} ${track}.webm`;
}

/**
 * The best format the browser can record, best first: VP9 compresses well and every browser that
 * records at all can write WebM; a recorder that supports none of these cannot be used.
 */
const CANDIDATES = ["video/webm;codecs=vp9,opus", "video/webm;codecs=vp8,opus", "video/webm;codecs=vp9", "video/webm;codecs=vp8", "video/webm"];

export function pickMime(isSupported: (mime: string) => boolean): string | null {
  return CANDIDATES.find(isSupported) ?? null;
}

/** What a recording is called in Zeno: "Recording 2026-11-21 14:05". Made from its start, so a finished-later upload names it the same. */
export function recordingTitle(startedAt: number): string {
  const d = new Date(startedAt);
  return `Recording ${d.getFullYear()}-${two(d.getMonth() + 1)}-${two(d.getDate())} ${two(d.getHours())}:${two(d.getMinutes())}`;
}
