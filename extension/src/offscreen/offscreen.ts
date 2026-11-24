// The offscreen document: the one place that holds the media streams and the recorders.
//
// A service worker cannot hold a stream, and a visible page would be closed by accident. An offscreen
// document is a page nobody sees, which the browser keeps alive for as long as it is using the camera or
// microphone. It does what the service worker tells it to, and reports what happened.

import { Api } from "../shared/api";
import { API_URL } from "../shared/config";
import { fileName, recordingTitle } from "../shared/format";
import { AFTER_COUNT_MS, COUNTDOWN_SECONDS, MAX_PAUSE_MS } from "../shared/limits";
import { readStored, saveManifest, saveUpload, trackFiles } from "../shared/library";
import { isOffscreenCommand, type OffscreenCommand, type OffscreenEvent, type SavedFile, type SavedNote, type Track, type UploadView } from "../shared/messages";
import { closeOff, judge, type UploadFile } from "../shared/recovery";
import { RecorderError, Session } from "./session";
import { PART_BYTES, Uploader, type Source } from "./uploader";

let session: Session | null = null;
/** The upload of the recording being made, which carries on after the recording has ended. */
let uploader: Uploader | null = null;
let ending: Promise<void> | null = null;
let urls: string[] = [];
/** A recording that is opening, or being counted down to: it can still be given up. */
interface Attempt {
  id: string;
  cancelled: boolean;
  /** The recorders have started: it can no longer be given up, only stopped. */
  began: boolean;
  /** Ends the wait of the count early. */
  wake: () => void;
}
let attempt: Attempt | null = null;
/** An earlier recording being finished. */
let recovering = false;
let pauseTimer: ReturnType<typeof setTimeout> | null = null;

function report(event: OffscreenEvent): void {
  void chrome.runtime.sendMessage(event).catch(() => undefined);
}

/** Puts text on the clipboard. An offscreen document may (reason CLIPBOARD); a popup that has closed cannot. */
function copyText(text: string): boolean {
  const box = document.createElement("textarea");
  box.value = text;
  document.body.append(box);
  box.select();
  let ok = false;
  try {
    ok = document.execCommand("copy");
  } catch {
    ok = false;
  }
  box.remove();
  return ok;
}

interface UploadJob {
  id: string;
  startedAt: number;
  track: Track;
  source: Source;
  partSize?: number;
  /** An earlier run began this upload: ask Zeno what it holds. */
  resume?: boolean;
}

/** What `upload.json` says, from how the upload stands. */
function uploadFileFor(view: UploadView, partSize: number): UploadFile {
  const status = view.phase === "done" ? "done" : view.phase === "off" ? "skipped" : view.phase === "failed" ? "failed" : "pending";
  return { version: 1, status, partSize };
}

/**
 * Starts uploading a recording, if there is anyone signed in to upload as. It never holds the recording up.
 * How the upload stands is kept beside the recording (`upload.json`), so that an upload that did not finish
 * can be finished at any later time, whatever became of the browser in between.
 */
function launchUpload(job: UploadJob): Uploader {
  let copied = false;
  const partSize = job.partSize ?? PART_BYTES;
  let kept: UploadFile["status"] | null = null;
  let checkpoint = Promise.resolve();
  const remember = (view: UploadView) => {
    const file = uploadFileFor(view, partSize);
    if (file.status === kept) return;
    kept = file.status;
    checkpoint = checkpoint.then(() => saveUpload(job.id, file)).catch(() => {
      if (kept === file.status) kept = null;
    });
  };
  remember({ phase: "connecting", sent: 0, total: null, url: null, copied: false });
  const next = new Uploader(
    {
      backend: new Api(API_URL),
      source: job.source,
      put: async (url, body) => {
        const reply = await fetch(url, { method: "PUT", body });
        if (!reply.ok) throw new Error(`storage answered ${reply.status}`);
      },
      sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
      emit: (view: UploadView) => {
        // The link is on the clipboard the moment the person stops (the size is known only then): the one
        // thing they do next is paste it.
        if (view.total !== null && view.url && !copied) copied = copyText(view.url);
        remember(view);
        report({ target: "background", type: "upload", id: job.id, view: { ...view, copied } });
      },
    },
    { recordingId: job.id, title: recordingTitle(job.startedAt), filename: fileName(job.track, job.startedAt), partSize, ...(job.resume ? { resume: true } : {}) },
  );
  next.start();
  return next;
}

function clearPauseTimer(): void {
  if (pauseTimer) clearTimeout(pauseTimer);
  pauseTimer = null;
}

/** Ends the session once, whoever asks first (the stop button, the browser's bar, a failing disk, a pause left too long). */
function finish(failure: RecorderError | null, note?: SavedNote): Promise<void> {
  if (ending) return ending;
  const current = session;
  if (!current) return Promise.resolve();
  clearPauseTimer();
  ending = (async () => {
    try {
      const { durationMs, files } = await current.stop();
      urls = files.map((file) => file.url);
      if (failure) report({ target: "background", type: "failed", id: current.id, code: failure.code, message: failure.message });
      else report({ target: "background", type: "stopped", id: current.id, durationMs, files, ...(note ? { note } : {}) });
    } catch (error) {
      const e = error instanceof RecorderError ? error : new RecorderError("unexpected", "The recording could not be finished.");
      report({ target: "background", type: "failed", id: current.id, code: failure?.code ?? e.code, message: failure?.message ?? e.message });
    } finally {
      // Whatever reached the disk is what is uploaded, and the link is handed over now, not when the upload is done.
      void uploader?.end(current.primary?.size() ?? 0);
      session = null;
      ending = null;
    }
  })();
  return ending;
}

/** Waits `ms`, or until the count is given up on. */
function waitOrCancel(ms: number, current: Attempt): Promise<void> {
  return new Promise((resolve) => {
    const timer = setTimeout(resolve, ms);
    current.wake = () => {
      clearTimeout(timer);
      resolve();
    };
  });
}

async function start(command: Extract<OffscreenCommand, { type: "start" }>): Promise<void> {
  if (session || attempt || recovering) return; // one recording at a time; the service worker will not ask twice
  for (const url of urls.splice(0)) URL.revokeObjectURL(url);
  const current: Attempt = { id: command.id, cancelled: false, began: false, wake: () => undefined };
  attempt = current;
  const giveUp = () => {
    current.cancelled = true;
    current.wake();
  };
  let prepared: Session | null = null;
  try {
    prepared = await Session.prepare(command.id, command.options, {
      // Before recording begins, ending the share is changing one's mind; after, it is the end.
      onSourceEnded: () => (current.began ? void finish(null) : giveUp()),
      onFailure: (error) => void finish(error),
      onWritten: () => uploader?.notify(),
    });
    if (command.options.countdown) {
      for (let remaining = COUNTDOWN_SECONDS; remaining > 0 && !current.cancelled; remaining -= 1) {
        report({ target: "background", type: "counting", id: command.id, remaining });
        await waitOrCancel(1000, current);
      }
      if (!current.cancelled) {
        // Say it is over, so the number comes off the page, and give the page a moment to show it gone.
        report({ target: "background", type: "counting", id: command.id, remaining: 0 });
        await new Promise((resolve) => setTimeout(resolve, AFTER_COUNT_MS));
      }
    }
    if (current.cancelled) {
      prepared.cancel();
      report({ target: "background", type: "failed", id: command.id, code: "cancelled", message: "Nothing was recorded." });
      return;
    }
    await prepared.begin();
    current.began = true;
    session = prepared;
    report({ target: "background", type: "started", id: command.id, startedAt: prepared.startedAt, tracks: prepared.tracks });
    const source = prepared.primary;
    uploader = source ? launchUpload({ id: prepared.id, startedAt: prepared.startedAt, track: source.track, source }) : null;
  } catch (error) {
    session = null;
    const e = error instanceof RecorderError ? error : new RecorderError("unexpected", "Something went wrong while starting.");
    report({ target: "background", type: "failed", id: command.id, code: e.code, message: e.message });
  } finally {
    if (attempt === current) attempt = null;
  }
}

function pause(id: string): void {
  if (!session || session.id !== id || ending) return;
  const at = session.pause();
  if (at === null) return;
  report({ target: "background", type: "paused", id, at });
  // A paused recording still holds the screen and the camera: one that is forgotten is stopped, and kept.
  clearPauseTimer();
  pauseTimer = setTimeout(() => void finish(null, "paused_too_long"), MAX_PAUSE_MS);
}

function resume(id: string): void {
  if (!session || session.id !== id || ending) return;
  const at = session.resume();
  if (at === null) return;
  clearPauseTimer();
  report({ target: "background", type: "resumed", id, at });
}

/** Finishes a recording an earlier run left behind: closes it off if it was cut short, and uploads what Zeno does not have. */
async function recover(id: string): Promise<void> {
  if (session || attempt || recovering) return;
  recovering = true;
  const fail = (message: string) => report({ target: "background", type: "failed", id, code: "unexpected", message });
  try {
    for (const url of urls.splice(0)) URL.revokeObjectURL(url);
    const stored = await readStored(id);
    const verdict = stored ? judge(stored, null) : null;
    if (!stored?.manifest || (verdict !== "interrupted" && verdict !== "upload" && verdict !== "finished")) {
      fail("There is nothing left to finish for that recording.");
      return;
    }
    let manifest = stored.manifest;
    const files = await trackFiles(id, manifest);
    const first = files[0];
    if (!first) {
      fail("The files of that recording are no longer on this computer.");
      return;
    }
    // Cut short: say how it ended, and hand the files to the browser as a recording that was stopped would.
    const wasCutShort = manifest.status === "recording";
    if (wasCutShort) {
      manifest = closeOff(manifest, stored.lastWriteAt, files.map((file) => file.file.size));
      await saveManifest(id, manifest);
    }
    const saved: (SavedFile & { url: string })[] = files.map((file) => {
      const url = URL.createObjectURL(file.file);
      urls.push(url);
      return { track: file.track, name: fileName(file.track, manifest.startedAt), bytes: file.file.size, mime: file.mime, url };
    });
    // Anything that was meant to go to Zeno and did not is sent now; a recording that was never meant to go (signed out) is not.
    const meant = stored.upload?.status === "pending" || stored.upload?.status === "failed";
    if (meant && first.track === manifest.tracks[0]?.track && first.file.size > 0) {
      uploader = launchUpload({
        id,
        startedAt: manifest.startedAt,
        track: first.track,
        source: { size: () => first.file.size, read: (start, end) => first.file.slice(start, end).arrayBuffer() },
        ...(stored.upload ? { partSize: stored.upload.partSize } : {}),
        resume: true,
      });
      void uploader.end(first.file.size);
    }
    report({
      target: "background",
      type: "stopped",
      id,
      durationMs: manifest.durationMs ?? 0,
      files: saved,
      ...(wasCutShort ? { note: "recovered" as const } : {}),
    });
  } catch {
    fail("That recording could not be finished.");
  } finally {
    recovering = false;
  }
}

chrome.runtime.onMessage.addListener((message: unknown) => {
  if (!isOffscreenCommand(message)) return false;
  switch (message.type) {
    case "start":
      void start(message);
      break;
    case "stop":
      if (session && session.id === message.id) void finish(null);
      break;
    case "pause":
      pause(message.id);
      break;
    case "resume":
      resume(message.id);
      break;
    case "cancel":
      if (attempt && attempt.id === message.id && !attempt.began) {
        attempt.cancelled = true;
        attempt.wake();
      }
      break;
    case "recover":
      void recover(message.id);
      break;
  }
  return false;
});
