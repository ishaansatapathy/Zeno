// The offscreen document: the one place that holds the media streams and the recorders.
//
// A service worker cannot hold a stream, and a visible page would be closed by accident. An offscreen
// document is a page nobody sees, which the browser keeps alive for as long as it is using the camera or
// microphone. It does what the service worker tells it to, and reports what happened.

import { Api } from "../shared/api";
import { API_URL } from "../shared/config";
import { fileName } from "../shared/format";
import { isOffscreenCommand, type OffscreenCommand, type OffscreenEvent, type UploadView } from "../shared/messages";
import { RecorderError, Session } from "./session";
import { Uploader } from "./uploader";

let session: Session | null = null;
/** The upload of the recording being made, which carries on after the recording has ended. */
let uploader: Uploader | null = null;
let ending: Promise<void> | null = null;
let urls: string[] = [];

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

/** Starts uploading what is being recorded, if there is anyone signed in to upload as. It never holds the recording up. */
function beginUpload(current: Session): Uploader | null {
  const source = current.primary;
  if (!source) return null;
  let copied = false;
  const stamp = new Date(current.startedAt);
  const title = `Recording ${stamp.getFullYear()}-${String(stamp.getMonth() + 1).padStart(2, "0")}-${String(stamp.getDate()).padStart(2, "0")} ${String(stamp.getHours()).padStart(2, "0")}:${String(stamp.getMinutes()).padStart(2, "0")}`;
  const next = new Uploader(
    {
      backend: new Api(API_URL),
      source,
      put: async (url, body) => {
        const reply = await fetch(url, { method: "PUT", body });
        if (!reply.ok) throw new Error(`storage answered ${reply.status}`);
      },
      sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
      emit: (view: UploadView) => {
        // The link is on the clipboard the moment the person stops (the size is known only then): the one
        // thing they do next is paste it.
        if (view.total !== null && view.url && !copied) copied = copyText(view.url);
        report({ target: "background", type: "upload", id: current.id, view: { ...view, copied } });
      },
    },
    { recordingId: current.id, title, filename: fileName(source.track, current.startedAt) },
  );
  next.start();
  return next;
}

/** Ends the session once, whoever asks first (the stop button, the browser's bar, a failing disk). */
function finish(failure: RecorderError | null): Promise<void> {
  if (ending) return ending;
  const current = session;
  if (!current) return Promise.resolve();
  ending = (async () => {
    try {
      const { durationMs, files } = await current.stop();
      urls = files.map((file) => file.url);
      if (failure) report({ target: "background", type: "failed", id: current.id, code: failure.code, message: failure.message });
      else report({ target: "background", type: "stopped", id: current.id, durationMs, files });
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

async function start(command: Extract<OffscreenCommand, { type: "start" }>): Promise<void> {
  if (session) return; // one recording at a time; the service worker will not ask twice
  for (const url of urls.splice(0)) URL.revokeObjectURL(url);
  try {
    session = await Session.start(command.id, command.options, {
      onSourceEnded: () => void finish(null),
      onFailure: (error) => void finish(error),
      onWritten: () => uploader?.notify(),
    });
    report({ target: "background", type: "started", id: command.id, startedAt: session.startedAt, tracks: session.tracks });
    uploader = beginUpload(session);
  } catch (error) {
    session = null;
    const e = error instanceof RecorderError ? error : new RecorderError("unexpected", "Something went wrong while starting.");
    report({ target: "background", type: "failed", id: command.id, code: e.code, message: e.message });
  }
}

chrome.runtime.onMessage.addListener((message: unknown) => {
  if (!isOffscreenCommand(message)) return false;
  if (message.type === "start") void start(message);
  else if (session && session.id === message.id) void finish(null);
  return false;
});
