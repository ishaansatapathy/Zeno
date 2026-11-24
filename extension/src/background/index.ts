// The service worker: owns the recorder's state and tells the other parts what to do.
//
// The browser may stop this worker whenever it is idle and start it again for the next message, so it
// remembers nothing between messages: the state lives in chrome.storage.session, every change goes
// through the pure `reduce`, and changes are applied one at a time so two messages cannot overwrite each
// other. The popup reads the same storage, so there is nothing to keep in step.

import { Api } from "../shared/api";
import { badgeFor } from "../shared/badge";
import { API_URL } from "../shared/config";
import { expired, toUnfinished, type Stored } from "../shared/recovery";
import { listStored, readStored, removeStored, saveDownloads } from "../shared/library";
import { isCommand, isOffscreenEvent, type Account, type Command, type OffscreenEvent, type RecordingOptions, type Unfinished } from "../shared/messages";
import { IDLE, isBusy, reduce, type RecorderEvent, type RecorderState } from "../shared/state";

const STATE_KEY = "recorder";
/** The tab the person was looking at when they started: the countdown and the camera bubble are drawn on it. */
const TAB_KEY = "recordingTab";
/** If the recorder has not answered in this long, it is not going to. */
const START_TIMEOUT_MS = 15_000;
/** Reading a recording back and closing it off takes a moment, not minutes. */
const RECOVER_TIMEOUT_MS = 30_000;
/** How many unfinished recordings the popup is told about. */
const MOST_UNFINISHED = 5;

async function load(): Promise<RecorderState> {
  const stored = await chrome.storage.session.get(STATE_KEY);
  return (stored[STATE_KEY] as RecorderState | undefined) ?? IDLE;
}

let queue: Promise<unknown> = Promise.resolve();

/** What the toolbar icon says: counting, recording, paused. It is there when the popup is not. */
function showBadge(state: RecorderState): void {
  const badge = badgeFor(state);
  void chrome.action.setBadgeText({ text: badge.text }).catch(() => undefined);
  if (badge.text) void chrome.action.setBadgeBackgroundColor({ color: badge.color }).catch(() => undefined);
}

/** Applies one event to the stored state, after the ones before it, and returns the new state. */
function apply(event: RecorderEvent): Promise<RecorderState> {
  const next = queue.then(async () => {
    const state = reduce(await load(), event);
    await chrome.storage.session.set({ [STATE_KEY]: state });
    showBadge(state);
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
 * What was written to disk is still there, and the start screen offers to finish it.
 */
async function reconcile(): Promise<RecorderState> {
  const state = await load();
  if (isBusy(state) && state.phase !== "choosing" && !(await offscreenOpen())) {
    await clearCountdown();
    return apply({ type: "failed", code: "unexpected", message: "The recording was interrupted. What was recorded before that is kept: you can finish it from the start screen." });
  }
  return state;
}

/** Who is signed in to Zeno. The extension has no sign-in of its own: it uses the web app's session. */
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

  if (tab?.id !== undefined) await chrome.storage.session.set({ [TAB_KEY]: tab.id });

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

async function tell(command: { type: "stop" | "pause" | "resume" | "cancel"; id: string }): Promise<void> {
  await chrome.runtime.sendMessage({ target: "offscreen", ...command }).catch(() => undefined);
}

async function stop(): Promise<RecorderState> {
  const state = await apply({ type: "stop-requested" });
  if (state.phase === "stopping") await tell({ type: "stop", id: state.id });
  return state;
}

/** Pausing and resuming are the recorder's to do: the state changes when it says it has (`paused`, `resumed`). */
async function hold(kind: "pause" | "resume"): Promise<RecorderState> {
  const state = await load();
  if (state.phase === "recording" && (kind === "pause") === (state.pausedAt === null)) await tell({ type: kind, id: state.id });
  return state;
}

async function cancel(): Promise<RecorderState> {
  const state = await load();
  if (state.phase === "counting") await tell({ type: "cancel", id: state.id });
  return state;
}

// ---- the page being shared: the countdown and the camera bubble ---------------------------------------------

async function tabId(): Promise<number | undefined> {
  const stored = await chrome.storage.session.get(TAB_KEY);
  return stored[TAB_KEY] as number | undefined;
}

/**
 * Drawn on the page by `executeScript`, so it can use nothing outside itself. A big number over a dimmed page,
 * and the way out: Esc. It lives in a shadow tree, so the page's own styles cannot reach it and it cannot reach them.
 */
function drawCountdown(remaining: number): void {
  const ID = "zeno-countdown";
  let host = document.getElementById(ID);
  if (!host) {
    host = document.createElement("div");
    host.id = ID;
    Object.assign(host.style, { all: "initial", position: "fixed", inset: "0", zIndex: "2147483647", pointerEvents: "none" });
    const root = host.attachShadow({ mode: "open" });
    const style = document.createElement("style");
    style.textContent =
      ".veil{position:fixed;inset:0;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:20px;background:rgba(0,0,0,.55);color:#f2f3f5;font-family:system-ui,sans-serif}" +
      ".n{font-size:160px;line-height:1;font-weight:500;font-variant-numeric:tabular-nums}" +
      ".hint{font-size:16px;color:#c9ccd1}";
    const veil = document.createElement("div");
    veil.className = "veil";
    const number = document.createElement("div");
    number.className = "n";
    number.setAttribute("role", "status");
    number.setAttribute("aria-live", "assertive");
    const hint = document.createElement("div");
    hint.className = "hint";
    hint.textContent = "Recording starts when it reaches zero. Press Esc to cancel.";
    veil.append(number, hint);
    root.append(style, veil);
    document.documentElement.append(host);
    const onKey = (event: KeyboardEvent) => {
      if (!document.getElementById(ID)) {
        window.removeEventListener("keydown", onKey, true);
        return;
      }
      if (event.key === "Escape") void chrome.runtime.sendMessage({ type: "recorder.cancel" });
    };
    window.addEventListener("keydown", onKey, true);
  }
  const number = host.shadowRoot?.querySelector(".n");
  if (number) number.textContent = String(remaining);
}

async function showCountdown(remaining: number): Promise<void> {
  const target = await tabId();
  if (target === undefined) return;
  await chrome.scripting.executeScript({ target: { tabId: target }, func: drawCountdown, args: [remaining] }).catch(() => undefined);
}

async function clearCountdown(): Promise<void> {
  const target = await tabId();
  if (target === undefined) return;
  await chrome.scripting.executeScript({ target: { tabId: target }, func: () => document.getElementById("zeno-countdown")?.remove() }).catch(() => undefined);
}

async function showBubble(): Promise<void> {
  const target = await tabId();
  if (target === undefined) return;
  await chrome.scripting.executeScript({ target: { tabId: target }, files: ["bubble-inject.js"] }).catch(() => undefined);
}

/** Takes the bubble off the page and forgets the page. The recording is over, however it ended. */
async function hideBubble(): Promise<void> {
  const target = await tabId();
  if (target === undefined) return;
  await chrome.storage.session.remove(TAB_KEY);
  await chrome.scripting
    .executeScript({
      target: { tabId: target },
      func: () => {
        document.getElementById("zeno-camera-bubble")?.remove();
        document.getElementById("zeno-countdown")?.remove();
      },
    })
    .catch(() => undefined);
}

// ---- recordings an earlier run left on this computer -----------------------------------------------------------

/** The id of the recording being made or finished right now, which must not be touched. */
async function liveId(): Promise<string | null> {
  const state = await load();
  return isBusy(state) && "id" in state ? state.id : null;
}

async function downloadState(id: number | undefined): Promise<"complete" | "in_progress" | "missing"> {
  if (!Number.isInteger(id)) return "missing";
  const [item] = await chrome.downloads.search({ id }).catch(() => []);
  if (item?.state === "complete" && item.exists) return "complete";
  return item?.state === "in_progress" ? "in_progress" : "missing";
}

const savingDownloads = new Map<string, Promise<void>>();

function saveFiles(event: Extract<OffscreenEvent, { type: "stopped" }>): Promise<void> {
  const existing = savingDownloads.get(event.id);
  if (existing) return existing;
  const saving = (async () => {
    const ids: NonNullable<Stored["downloadIds"]> = { ...(await readStored(event.id))?.downloadIds };
    for (const file of event.files) {
      if (await downloadState(ids[file.track]) !== "missing") continue;
      const id = await chrome.downloads.download({ url: file.url, filename: `Zeno/${file.name}`, conflictAction: "uniquify" }).catch(() => undefined);
      if (id === undefined) continue;
      ids[file.track] = id;
      await saveDownloads(event.id, ids);
    }
  })().finally(() => savingDownloads.delete(event.id));
  savingDownloads.set(event.id, saving);
  return saving;
}

/** What is left unfinished (newest first), after clearing away what is old and done with. */
async function unfinished(): Promise<Unfinished[]> {
  await reconcile();
  const live = await liveId();
  const now = Date.now();
  const kept: Unfinished[] = [];
  for (const stored of await listStored()) {
    if (stored.id === live || savingDownloads.has(stored.id)) continue;
    const tracks = stored.manifest?.status === "complete" ? stored.manifest.tracks : [];
    const states = await Promise.all(tracks.map(({ track }) => downloadState(stored.downloadIds?.[track])));
    const downloaded = states.length > 0 && states.every((state) => state === "complete");
    if (expired(stored, live, now, downloaded)) {
      await removeStored(stored.id).catch(() => undefined);
    } else {
      const item = toUnfinished(stored, live, states.includes("missing"));
      if (item) kept.push(item);
    }
  }
  return kept.sort((a, b) => b.startedAt - a.startedAt).slice(0, MOST_UNFINISHED);
}

async function recover(id: string): Promise<RecorderState> {
  await reconcile();
  const requested = await apply({ type: "recover-requested", id });
  if (requested.phase !== "recovering" || requested.id !== id) return requested;
  try {
    await ensureOffscreen();
    await chrome.runtime.sendMessage({ target: "offscreen", type: "recover", id });
  } catch {
    return apply({ type: "failed", code: "unexpected", message: "The recording could not be finished." });
  }
  setTimeout(() => {
    void load().then((state) => {
      if (state.phase === "recovering" && state.id === id) void apply({ type: "failed", code: "unexpected", message: "The recording could not be finished. Try again." });
    });
  }, RECOVER_TIMEOUT_MS);
  return requested;
}

async function discard(id: string): Promise<Unfinished[]> {
  // Never what is being recorded or finished right now.
  if ((await liveId()) !== id && !savingDownloads.has(id)) await removeStored(id).catch(() => undefined);
  return unfinished();
}

// ---- what the recorder reports ----------------------------------------------------------------------------

async function onRecorder(event: OffscreenEvent): Promise<void> {
  switch (event.type) {
    case "counting": {
      const state = await apply({ type: "counting", id: event.id, remaining: event.remaining });
      if (state.phase === "counting" && state.remaining > 0) await showCountdown(state.remaining);
      else await clearCountdown();
      return;
    }
    case "started": {
      const state = await apply({ type: "started", id: event.id, startedAt: event.startedAt, tracks: event.tracks });
      await clearCountdown();
      if (state.phase === "recording" && event.tracks.includes("camera") && event.tracks.includes("screen")) await showBubble();
      return;
    }
    case "paused":
      await apply({ type: "paused", id: event.id, at: event.at });
      return;
    case "resumed":
      await apply({ type: "resumed", id: event.id, at: event.at });
      return;
    case "stopped": {
      await hideBubble();
      const files = event.files.map(({ url: _url, ...file }) => file);
      const state = await apply({ type: "stopped", id: event.id, durationMs: event.durationMs, files, ...(event.note ? { note: event.note } : {}) });
      if (state.phase === "saved" && state.id === event.id) {
        await saveFiles(event).catch(() => undefined);
      }
      return;
    }
    case "upload":
      await apply({ type: "upload", id: event.id, view: event.view });
      return;
    case "failed":
      await hideBubble();
      // Closing the picker, or giving up the count, is not a failure: go back to the start without a word.
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
      case "recorder.pause":
        return hold("pause");
      case "recorder.resume":
        return hold("resume");
      case "recorder.cancel":
        return cancel();
      case "recorder.dismiss":
        return apply({ type: "dismissed" });
      case "recorder.unfinished":
        return unfinished();
      case "recorder.recover":
        return recover(command.id);
      case "recorder.discard":
        return discard(command.id);
    }
  })().then(sendResponse);
  return true; // the answer is sent later
});

// A recording does not survive the browser closing. If one was under way, say so at the next start.
chrome.runtime.onStartup.addListener(() => void reconcile());
