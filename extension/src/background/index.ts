// The service worker: owns the recorder's state and tells the other parts what to do.
//
// The browser may stop this worker whenever it is idle and start it again for the next message, so it
// remembers nothing between messages: the state lives in chrome.storage.session, every change goes
// through the pure `reduce`, and changes are applied one at a time so two messages cannot overwrite each
// other. The popup reads the same storage, so there is nothing to keep in step.

import { Api } from "../shared/api";
import { API_URL } from "../shared/config";
import { isCommand, isOffscreenEvent, type Account, type Command, type OffscreenEvent, type RecordingOptions } from "../shared/messages";
import { IDLE, isBusy, reduce, type RecorderEvent, type RecorderState } from "../shared/state";

const STATE_KEY = "recorder";
/** Where the camera bubble was put, so it can be taken away again. */
const BUBBLE_KEY = "bubbleTab";
/** If the recorder has not answered in this long, it is not going to. */
const START_TIMEOUT_MS = 15_000;

async function load(): Promise<RecorderState> {
  const stored = await chrome.storage.session.get(STATE_KEY);
  return (stored[STATE_KEY] as RecorderState | undefined) ?? IDLE;
}

let queue: Promise<unknown> = Promise.resolve();

/** Applies one event to the stored state, after the ones before it, and returns the new state. */
function apply(event: RecorderEvent): Promise<RecorderState> {
  const next = queue.then(async () => {
    const state = reduce(await load(), event);
    await chrome.storage.session.set({ [STATE_KEY]: state });
    return state;
  });
  queue = next.catch(() => undefined);
  return next;
}

async function offscreenOpen(): Promise<boolean> {
  const contexts = await chrome.runtime.getContexts({ contextTypes: [chrome.runtime.ContextType.OFFSCREEN_DOCUMENT] });
  return contexts.length > 0;
}

async function ensureOffscreen(): Promise<void> {
  if (await offscreenOpen()) return;
  await chrome.offscreen.createDocument({
    url: "offscreen.html",
    reasons: [chrome.offscreen.Reason.USER_MEDIA, chrome.offscreen.Reason.DISPLAY_MEDIA, chrome.offscreen.Reason.CLIPBOARD],
    justification: "Record the screen, camera and microphone while the popup is closed, and put the share link on the clipboard when it ends.",
  });
}

/**
 * If the state says a recording is under way but there is nothing recording (the browser was closed or
 * crashed, the offscreen document was killed), say so instead of showing a recorder that is not there.
 * What was written to disk is still there.
 */
async function reconcile(): Promise<RecorderState> {
  const state = await load();
  if (isBusy(state) && state.phase !== "choosing" && !(await offscreenOpen())) {
    return apply({ type: "failed", code: "unexpected", message: "The recording was interrupted. What was recorded before that is kept." });
  }
  return state;
}

/** Who is signed in to Zeno in this browser. The extension has no sign-in of its own: it uses the web app's session. */
async function account(): Promise<Account> {
  const me = await new Api(API_URL).me();
  if (me.ok) return { status: "signed_in", name: me.data.name, email: me.data.email ?? "" };
  return me.error.status === 0 || me.error.status >= 500 ? { status: "unreachable" } : { status: "signed_out" };
}

async function start(options: RecordingOptions): Promise<RecorderState> {
  const id = crypto.randomUUID();
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  const requested = await apply({ type: "start-requested", id, screen: options.screen });
  // Busy or not at rest: this one is refused, and the state that is already there is what the caller sees.
  if (requested.phase !== "choosing" && requested.phase !== "starting") return requested;
  if (requested.id !== id) return requested;

  if (options.camera && options.screen && tab?.id !== undefined) await chrome.storage.session.set({ [BUBBLE_KEY]: tab.id });

  try {
    await ensureOffscreen();
    // The recorder opens the browser's own picker itself (when the screen is wanted), and answers with
    // "started", "failed", or "cancelled" if the person closed the picker without choosing.
    await chrome.runtime.sendMessage({ target: "offscreen", type: "start", id, options });
  } catch {
    return apply({ type: "failed", code: "unexpected", message: "The recorder could not be started." });
  }
  // The recorder answers with "started" or "failed". If it answers with neither, it will not.
  setTimeout(() => {
    void load().then((state) => {
      if (state.phase === "starting" && state.id === id) void apply({ type: "failed", code: "unexpected", message: "The recorder did not start. Try again." });
    });
  }, START_TIMEOUT_MS);
  return load();
}

async function stop(): Promise<RecorderState> {
  const state = await apply({ type: "stop-requested" });
  if (state.phase === "stopping") await chrome.runtime.sendMessage({ target: "offscreen", type: "stop", id: state.id }).catch(() => undefined);
  return state;
}

// ---- the camera bubble ----------------------------------------------------------------------------------

async function showBubble(): Promise<void> {
  const stored = await chrome.storage.session.get(BUBBLE_KEY);
  const tabId = stored[BUBBLE_KEY] as number | undefined;
  if (tabId === undefined) return;
  await chrome.scripting.executeScript({ target: { tabId }, files: ["bubble-inject.js"] }).catch(() => undefined);
}

async function hideBubble(): Promise<void> {
  const stored = await chrome.storage.session.get(BUBBLE_KEY);
  const tabId = stored[BUBBLE_KEY] as number | undefined;
  if (tabId === undefined) return;
  await chrome.storage.session.remove(BUBBLE_KEY);
  await chrome.scripting.executeScript({ target: { tabId }, func: () => document.getElementById("zeno-camera-bubble")?.remove() }).catch(() => undefined);
}

// ---- what the recorder reports ----------------------------------------------------------------------------

async function onRecorder(event: OffscreenEvent): Promise<void> {
  switch (event.type) {
    case "started": {
      const state = await apply({ type: "started", id: event.id, startedAt: event.startedAt, tracks: event.tracks });
      if (state.phase === "recording" && event.tracks.includes("camera") && event.tracks.includes("screen")) await showBubble();
      return;
    }
    case "stopped": {
      await hideBubble();
      const files = event.files.map(({ url: _url, ...file }) => file);
      const state = await apply({ type: "stopped", id: event.id, durationMs: event.durationMs, files });
      // Only a recording that really ended is saved to the person's disk; the copy in the extension's own
      // storage stays, so it can be uploaded (or recovered) later.
      if (state.phase === "saved" && state.id === event.id) {
        for (const file of event.files) await chrome.downloads.download({ url: file.url, filename: `Zeno/${file.name}`, conflictAction: "uniquify" }).catch(() => undefined);
      }
      return;
    }
    case "upload":
      await apply({ type: "upload", id: event.id, view: event.view });
      return;
    case "failed":
      await hideBubble();
      // Closing the picker is not a failure: go back to the start without a word.
      if (event.code === "cancelled") await apply({ type: "picker-cancelled" });
      else await apply({ type: "failed", code: event.code, message: event.message });
      return;
  }
}

chrome.runtime.onMessage.addListener((message: unknown, _sender, sendResponse) => {
  if (isOffscreenEvent(message)) {
    void onRecorder(message);
    return false;
  }
  if (!isCommand(message)) return false;
  const command: Command = message;
  void (async () => {
    switch (command.type) {
      case "recorder.status":
        return reconcile();
      case "recorder.account":
        return account();
      case "recorder.start":
        await reconcile();
        return start(command.options);
      case "recorder.stop":
        return stop();
      case "recorder.dismiss":
        return apply({ type: "dismissed" });
    }
  })().then(sendResponse);
  return true; // the answer is sent later
});

// A recording does not survive the browser closing. If one was under way, say so at the next start.
chrome.runtime.onStartup.addListener(() => void reconcile());
