// Which recordings left on this computer need finishing, and which can be thrown away.
//
// A recording is kept in the extension's own storage while it is made and after: the files, a
// `manifest.json` that says how it stands (`recording` until it is stopped, `complete` after) and, for one
// that was uploaded, an `upload.json` that says how the upload went. From those three the extension can tell,
// at any later time, what happened to it, so nothing depends on the browser having stayed open. All of it is
// plain data in and plain answers out, so it is tested without a browser.

import { activeMs, joints, type Joint, type Pause } from "./pauses";
import type { Unfinished } from "./messages";

export interface Manifest {
  version: number;
  id: string;
  /** `recording` until the recorder was stopped: if it stays so, the recording was cut short. */
  status: "recording" | "complete";
  startedAt: number;
  tracks: { track: "screen" | "camera"; file: string; mime: string; startedAt: number }[];
  /** Every pause, in wall-clock time. */
  pauses?: Pause[];
  endedAt?: number;
  durationMs?: number;
  /** Set when the recording was cut short and was closed off afterwards. */
  recovered?: boolean;
}

/** How the upload of a recording went, as far as the extension knows. */
export interface UploadFile {
  version: 1;
  /** `pending`: begun, not finished. `failed`: gave up. `done`: all in Zeno. `skipped`: it was never going to be uploaded (signed out, not allowed). */
  status: "pending" | "failed" | "done" | "skipped";
  partSize: number;
}

/** What is on the disk for one recording. */
export interface Stored {
  id: string;
  manifest: Manifest | null;
  upload: UploadFile | null;
  downloadIds?: Partial<Record<"screen" | "camera", number>>;
  emptyVerified?: boolean;
  /** The size of the file that is uploaded (the first track). */
  bytes: number;
  /** When that file was last written to: for a recording cut short, the last thing that reached the disk. */
  lastWriteAt: number;
}

export type Verdict =
  /** Being recorded right now: not to be touched. */
  | "live"
  /** Cut short, with something worth keeping. */
  | "interrupted"
  /** Stopped, but not all in Zeno, and it is meant to be. */
  | "upload"
  /** Done with: kept for a while, in case it is wanted again. */
  | "finished"
  /** Nothing in it worth keeping (the browser went in the first moment). */
  | "empty"
  /** Cannot be understood: left alone, never deleted. */
  | "unknown";

export function judge(stored: Stored, liveId: string | null): Verdict {
  if (stored.id === liveId) return "live";
  const { manifest } = stored;
  if (!manifest) return stored.bytes === 0 ? "empty" : "unknown";
  if (manifest.status === "recording") return stored.bytes === 0 ? "empty" : "interrupted";
  if (stored.upload && (stored.upload.status === "pending" || stored.upload.status === "failed")) return "upload";
  return "finished";
}

/** How long a recording that is done with is kept on this computer (it has also been saved to Downloads). */
export const KEEP_FINISHED_MS = 7 * 24 * 3600 * 1000;
/** How long an empty one is kept before it is swept away. */
export const KEEP_EMPTY_MS = 24 * 3600 * 1000;

/** Whether a recording can be deleted without anybody losing anything. Never one that could still be finished. */
export function expired(stored: Stored, liveId: string | null, now: number, downloaded = false): boolean {
  const verdict = judge(stored, liveId);
  const last = stored.manifest?.endedAt ?? stored.lastWriteAt;
  if (verdict === "finished") return downloaded && now - last > KEEP_FINISHED_MS;
  if (verdict === "empty") return stored.emptyVerified === true && now - stored.lastWriteAt > KEEP_EMPTY_MS;
  return false;
}

/** How long a recording that was cut short ran: from its start to the last thing written, less the pauses. */
export function interruptedLength(manifest: Manifest, lastWriteAt: number): number {
  return activeMs(manifest.startedAt, Math.max(manifest.startedAt, lastWriteAt), manifest.pauses ?? []);
}

/** How the recording should be described to the person, or `null` if there is nothing to offer. Newest first is the caller's job. */
export function toUnfinished(stored: Stored, liveId: string | null, needsDownload = false): Unfinished | null {
  const verdict = judge(stored, liveId);
  const { manifest } = stored;
  if (!manifest || (verdict !== "interrupted" && verdict !== "upload" && !(verdict === "finished" && needsDownload))) return null;
  return {
    id: stored.id,
    startedAt: manifest.startedAt,
    durationMs: verdict === "interrupted" ? interruptedLength(manifest, stored.lastWriteAt) : (manifest.durationMs ?? interruptedLength(manifest, stored.lastWriteAt)),
    bytes: stored.bytes,
    kind: verdict === "finished" ? "download" : verdict,
  };
}

/**
 * The manifest of a recording that was cut short, closed off: what it should have said had it been stopped. The
 * end is the last thing that reached the disk, which is as close as anything can say.
 */
export function closeOff(manifest: Manifest, lastWriteAt: number, bytes: number[]): Manifest & { bytes: number[]; joints: Joint[] } {
  const endedAt = Math.max(manifest.startedAt, lastWriteAt);
  // A pause that was still going on when it was cut short ends where the recording does.
  const pauses = (manifest.pauses ?? []).map((pause) => (pause.to === null ? { from: pause.from, to: Math.max(pause.from, endedAt) } : pause));
  return {
    ...manifest,
    status: "complete",
    pauses,
    endedAt,
    durationMs: activeMs(manifest.startedAt, endedAt, pauses),
    joints: joints(manifest.startedAt, pauses, endedAt),
    recovered: true,
    bytes,
  };
}
