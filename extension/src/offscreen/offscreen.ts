// The offscreen document: the one place that holds the media streams and the recorders.
//
// A service worker cannot hold a stream, and a visible page would be closed by accident. An offscreen
// document is a page nobody sees, which the browser keeps alive for as long as it is using the camera or
// microphone. It does what the service worker tells it to, and reports what happened.

import { isOffscreenCommand, type OffscreenCommand, type OffscreenEvent } from "../shared/messages";
import { RecorderError, Session } from "./session";

let session: Session | null = null;
let ending: Promise<void> | null = null;
let urls: string[] = [];

function report(event: OffscreenEvent): void {
  void chrome.runtime.sendMessage(event).catch(() => undefined);
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
    });
    report({ target: "background", type: "started", id: command.id, startedAt: session.startedAt, tracks: session.tracks });
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
