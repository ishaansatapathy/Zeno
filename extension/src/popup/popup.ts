// What the person sees. It holds no recording and decides nothing: it shows the state the service worker
// keeps (and re-shows it whenever that changes, so closing and reopening the popup loses nothing) and
// sends the person's choices as commands.

import { bytesText, clock } from "../shared/format";
import { WEB_URL } from "../shared/config";
import type { Account, Command, RecordingOptions, SavedFile, Unfinished, UploadView } from "../shared/messages";
import { elapsedMs, IDLE, type RecorderState, type Running } from "../shared/state";

const app = document.getElementById("app") as HTMLElement;
const STATE_KEY = "recorder";
const SETTINGS_KEY = "settings";

interface Settings {
  screen: boolean;
  systemAudio: boolean;
  microphone: boolean;
  camera: boolean;
  microphoneId?: string;
  cameraId?: string;
  countdown: boolean;
}

const DEFAULTS: Settings = { screen: true, systemAudio: false, microphone: true, camera: false, countdown: true };
let settings: Settings = { ...DEFAULTS };
let timer: ReturnType<typeof setInterval> | null = null;
let current: RecorderState = IDLE;
/** What is on screen, apart from the numbers that change as an upload goes: if it is the same, only the numbers are updated. */
let shownKey = "";

// ---- a very small way to build elements, without ever turning text into markup ------------------------------

type Child = Node | string | null | false;

/** Attributes are set as attributes; everything else (type, id, checked, disabled, value, ...) as a property. */
type Props = Record<string, string | boolean | undefined | null>;

function h<K extends keyof HTMLElementTagNameMap>(tag: K, props: Props = {}, ...children: Child[]): HTMLElementTagNameMap[K] {
  const element = document.createElement(tag);
  for (const [key, value] of Object.entries(props)) {
    if (value === undefined || value === null || value === false) continue;
    if (key === "class") element.className = String(value);
    else if (key === "role" || key === "style" || key.startsWith("aria-")) element.setAttribute(key, String(value));
    else (element as unknown as Record<string, unknown>)[key] = value;
  }
  for (const child of children) if (child) element.append(child);
  return element;
}

async function ask<T>(command: Command): Promise<T> {
  return (await chrome.runtime.sendMessage(command)) as T;
}

function send(command: Command): Promise<RecorderState> {
  return ask<RecorderState>(command);
}

// ---- remembering what the person chose last time ---------------------------------------------------------

async function loadSettings(): Promise<void> {
  const stored = await chrome.storage.local.get(SETTINGS_KEY);
  settings = { ...DEFAULTS, ...(stored[SETTINGS_KEY] as Partial<Settings> | undefined) };
}

function save(): void {
  void chrome.storage.local.set({ [SETTINGS_KEY]: settings });
}

// ---- the camera and microphone ---------------------------------------------------------------------------

async function granted(name: "camera" | "microphone"): Promise<boolean> {
  try {
    return (await navigator.permissions.query({ name: name as PermissionName })).state === "granted";
  } catch {
    return false;
  }
}

/** Devices the browser will name for us: it only does so once permission has been given. */
async function devices(kind: MediaDeviceKind): Promise<MediaDeviceInfo[]> {
  try {
    return (await navigator.mediaDevices.enumerateDevices()).filter((device) => device.kind === kind && device.label);
  } catch {
    return [];
  }
}

// ---- the screens --------------------------------------------------------------------------------------------

function option(id: keyof Settings, title: string, hint: string, opts: { sub?: boolean; disabled?: boolean } = {}): HTMLLIElement {
  const input = h("input", { type: "checkbox", id, checked: Boolean(settings[id]), disabled: opts.disabled });
  input.addEventListener("change", () => {
    (settings[id] as boolean) = input.checked;
    save();
    render(current);
  });
  const label = h(
    "label",
    { class: `option${opts.sub ? " sub" : ""}`, "aria-disabled": opts.disabled ? "true" : undefined },
    input,
    h("span", { class: "box" }),
    h("span", { class: "text" }, title, h("span", { class: "hint" }, hint)),
  );
  return h("li", {}, label);
}

function deviceSelect(id: "microphoneId" | "cameraId", label: string, list: MediaDeviceInfo[]): HTMLSelectElement | null {
  if (list.length < 2) return null;
  const select = h("select", { id, "aria-label": label });
  for (const device of list) select.append(h("option", { value: device.deviceId, selected: settings[id] === device.deviceId }, device.label));
  select.addEventListener("change", () => {
    settings[id] = select.value;
    save();
  });
  return select;
}

// ---- the share link ---------------------------------------------------------------------------------------

/** Who is signed in. A server that is slow to answer must not hold the popup up, so it is not waited for. */
async function whoIsSignedIn(): Promise<Account | null> {
  try {
    return await Promise.race([ask<Account>({ type: "recorder.account" }), new Promise<null>((resolve) => setTimeout(() => resolve(null), 1500))]);
  } catch {
    return null;
  }
}

function openZeno(path: string): void {
  void chrome.tabs.create({ url: `${WEB_URL}${path}` });
}

function accountNote(account: Account | null): HTMLElement | null {
  if (!account) return null;
  if (account.status === "signed_in") return h("p", { class: "note", id: "account" }, `A share link is made as soon as you start. Signed in as ${account.email || account.name}.`);
  if (account.status === "unreachable") return h("p", { class: "note", id: "account" }, "Zeno cannot be reached right now. You can still record: it is saved on this computer.");
  const signIn = h("button", { class: "secondary", id: "sign-in", type: "button" }, "Sign in to Zeno");
  signIn.addEventListener("click", () => openZeno("/login"));
  return h("div", { id: "account", class: "account" }, h("p", { class: "note" }, "Sign in to Zeno to get a share link while you record. Without it, the recording is only saved on this computer."), signIn);
}

/** One line on how the upload is going, in words a person can act on. */
function uploadText(view: UploadView | undefined): string {
  if (!view) return "";
  switch (view.phase) {
    case "off":
      return view.reason === "signed_out" ? "Not uploaded: sign in to Zeno to get a link." : (view.message ?? "Not uploaded.");
    case "connecting":
      return view.reason === "unreachable" ? "Zeno cannot be reached. Recording goes on, and the upload catches up." : "Getting your link ready…";
    case "uploading":
      return view.reason === "unreachable" ? "Zeno cannot be reached. Recording goes on, and the upload catches up." : `Uploading as you record · ${bytesText(view.sent)} sent`;
    case "finishing":
      return view.reason === "unreachable" ? "Zeno cannot be reached. Trying again…" : "Finishing the upload…";
    case "done":
      return "Uploaded. Zeno is preparing it for playback.";
    case "failed":
      return `${view.message ?? "The upload did not finish."} Your copy is saved on this computer.`;
  }
}

/** The link, ready to copy, and how the upload is going. */
function linkBox(view: UploadView | undefined): HTMLElement | null {
  if (!view) return null;
  const parts: Child[] = [];
  if (view.url) {
    const url = view.url;
    const field = h("input", { class: "link", id: "share-url", type: "text", readOnly: true, value: url, "aria-label": "Share link" });
    field.addEventListener("focus", () => field.select());
    const copy = h("button", { class: "secondary copy", id: "copy", type: "button" }, "Copy link");
    copy.addEventListener("click", () => {
      void navigator.clipboard.writeText(url).then(() => {
        copy.textContent = "Copied";
      });
    });
    parts.push(h("div", { class: "linkrow" }, field, copy));
    if (view.copied) parts.push(h("p", { class: "note", id: "copied" }, "The link is on your clipboard."));
  }
  parts.push(h("p", { class: "note", id: "upload-line", role: "status", "aria-live": "polite" }, uploadText(view)));
  return h("div", { class: "share" }, ...parts);
}

// ---- recordings an earlier run left unfinished ---------------------------------------------------------------

/** What is left on this computer to finish. A slow answer must not hold the start screen up, so it is not waited for long. */
async function whatIsUnfinished(): Promise<Unfinished[]> {
  try {
    const list = await Promise.race([ask<Unfinished[]>({ type: "recorder.unfinished" }), new Promise<null>((resolve) => setTimeout(() => resolve(null), 2000))]);
    return list ?? [];
  } catch {
    return [];
  }
}

function when(at: number): string {
  return new Date(at).toLocaleString([], { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
}

function unfinishedList(items: Unfinished[]): HTMLElement | null {
  if (items.length === 0) return null;
  const rows = items.map((item) => {
    const label = when(item.startedAt);
    const action = {
      interrupted: { label: "Recover", description: "Recover the recording from", status: "cut short" },
      upload: { label: "Finish upload", description: "Finish the upload of the recording from", status: "not all in Zeno yet" },
      download: { label: "Save locally", description: "Save a local copy of the recording from", status: "local save incomplete" },
    }[item.kind];
    const finish = h(
      "button",
      { class: "secondary small", type: "button", "data-action": "recover", "aria-label": `${action.description} ${label}` },
      action.label,
    );
    finish.addEventListener("click", () => {
      finish.disabled = true;
      void send({ type: "recorder.recover", id: item.id });
    });
    // Deleting for good takes two presses, and the second says what it will do.
    const discard = h("button", { class: "quiet", type: "button", "data-action": "discard", "aria-label": `Delete the recording from ${label} from this computer` }, "Delete");
    let armed = false;
    const name = discard.getAttribute("aria-label") ?? "";
    discard.addEventListener("click", () => {
      if (!armed) {
        armed = true;
        discard.textContent = "Delete for good?";
        discard.setAttribute("aria-label", `Press again to delete the recording from ${label} for good`);
        return;
      }
      discard.disabled = true;
      void ask<Unfinished[]>({ type: "recorder.discard", id: item.id }).then(() => render(current, true));
    });
    discard.addEventListener("blur", () => {
      armed = false;
      discard.textContent = "Delete";
      discard.setAttribute("aria-label", name);
    });
    return h(
      "li",
      {},
      h(
        "div",
        { class: "what" },
        h("span", { class: "when" }, label),
        h("span", { class: "meta" }, `${clock(item.durationMs)} · ${bytesText(item.bytes)} · ${action.status}`),
      ),
      h("div", { class: "actions" }, finish, discard),
    );
  });
  return h("section", { class: "unfinished", "aria-label": "Unfinished recordings" }, h("p", { class: "label" }, "Unfinished"), h("ul", {}, ...rows));
}

async function idleScreen(): Promise<HTMLElement> {
  const [account, unfinished] = await Promise.all([whoIsSignedIn(), whatIsUnfinished()]);
  const microphones = settings.microphone ? await devices("audioinput") : [];
  const cameras = settings.camera ? await devices("videoinput") : [];
  const items = [
    option("screen", "Screen", "A tab, a window or the whole screen"),
    option("systemAudio", "Sound from the screen", "Tick Share audio in the next step too", { sub: true, disabled: !settings.screen }),
    option("microphone", "Microphone", "Your voice"),
    option("camera", "Camera", "Shown over the screen and saved as its own file"),
    option("countdown", "Countdown", "3, 2, 1 before it starts"),
  ];
  const list = h("ul", { class: "options" }, ...items);
  const micSelect = deviceSelect("microphoneId", "Microphone", microphones);
  const camSelect = deviceSelect("cameraId", "Camera", cameras);
  if (micSelect) items[2]?.append(micSelect);
  if (camSelect) items[3]?.append(camSelect);

  const start = h("button", { class: "primary", id: "start", type: "button" }, "Start recording");
  start.disabled = !settings.screen && !settings.camera;
  start.addEventListener("click", () => void begin(start));
  return h(
    "section",
    {},
    h("h1", {}, "Ready when ", h("em", {}, "you are.")),
    unfinishedList(unfinished),
    list,
    start,
    accountNote(account),
    h("p", { class: "note" }, "Recordings are saved to your Downloads folder, in a Zeno folder."),
  );
}

function begin(button: HTMLButtonElement): Promise<void> {
  return (async () => {
    // The browser asks for the camera and microphone with a prompt a popup cannot show. A page can.
    const missing = (settings.microphone && !(await granted("microphone"))) || (settings.camera && !(await granted("camera")));
    if (missing) {
      const query = new URLSearchParams({ microphone: String(settings.microphone), camera: String(settings.camera) });
      await chrome.tabs.create({ url: chrome.runtime.getURL(`permissions.html?${query}`) });
      window.close();
      return;
    }
    button.disabled = true;
    const options: RecordingOptions = { ...settings, systemAudio: settings.screen && settings.systemAudio };
    await send({ type: "recorder.start", options });
  })();
}

function message(title: Child, body: string, role: "status" | "alert" = "status"): HTMLElement {
  return h("section", {}, h("h1", {}, title), h("p", { role, "aria-live": "polite", class: "note" }, body));
}

function countingScreen(state: Extract<RecorderState, { phase: "counting" }>): HTMLElement {
  const cancel = h("button", { class: "secondary", id: "cancel", type: "button" }, "Cancel");
  cancel.addEventListener("click", () => {
    cancel.disabled = true;
    void send({ type: "recorder.cancel" });
  });
  return h(
    "section",
    {},
    h("h1", {}, state.remaining > 0 ? h("span", {}, "Starting in ", h("em", { id: "count" }, String(state.remaining))) : "Starting…"),
    h("p", { class: "note", role: "status", "aria-live": "polite", style: "margin:0 0 18px" }, "Get ready. Nothing is recorded until it reaches zero. Esc on the page cancels."),
    cancel,
  );
}

function recordingScreen(state: Running): HTMLElement {
  const paused = state.pausedAt !== null;
  const time = h("span", { class: "clock", id: "clock" }, clock(elapsedMs(state, Date.now())));
  const stop = h("button", { class: "primary", id: "stop", type: "button" }, "Stop and save");
  stop.addEventListener("click", () => {
    stop.disabled = true;
    void send({ type: "recorder.stop" });
  });
  // It says so when it has paused or resumed, not when the button was pressed: until then the button waits.
  const hold = h("button", { class: "secondary hold", id: paused ? "resume" : "pause", type: "button" }, paused ? "Resume" : "Pause");
  hold.addEventListener("click", () => {
    hold.disabled = true;
    void send({ type: paused ? "recorder.resume" : "recorder.pause" });
  });
  if (!paused) {
    timer = setInterval(() => {
      time.textContent = clock(elapsedMs(state, Date.now()));
    }, 250);
  }
  const names = state.tracks.map((track) => (track === "screen" ? "Screen" : "Camera")).join(" and ");
  return h(
    "section",
    {},
    h("div", { class: `status${paused ? " paused" : ""}`, role: "status" }, h("span", { class: "dot", "aria-hidden": "true" }), time),
    h("p", { class: "label", id: "recording-label", style: "margin-bottom:20px" }, paused ? `Paused · ${names}` : `Recording ${names}`),
    linkBox(state.upload),
    hold,
    stop,
    h("p", { class: "note" }, "You can also stop from the browser's own Stop sharing bar."),
  );
}

/** Said when a recording did not end because the person pressed stop. */
function noteText(note: "recovered" | "paused_too_long" | undefined): string | null {
  if (note === "recovered") return "This recording was cut short. What had reached the disk was kept, and is saved here.";
  if (note === "paused_too_long") return "It was left paused for a long time, so it was stopped and saved: a paused recording still has the screen and camera shared.";
  return null;
}

function savedScreen(state: Extract<RecorderState, { phase: "saved" }>): HTMLElement {
  const again = h("button", { class: "secondary", id: "again", type: "button" }, "Record again");
  again.addEventListener("click", () => void send({ type: "recorder.dismiss" }));
  // The copy on this computer is whole, so an upload that did not finish can be tried again at any time.
  const retry =
    state.upload?.phase === "failed"
      ? h("button", { class: "secondary", id: "retry-upload", type: "button", style: "margin-bottom:10px" }, "Try the upload again")
      : null;
  retry?.addEventListener("click", () => {
    retry.disabled = true;
    void send({ type: "recorder.recover", id: state.id });
  });
  const why = noteText(state.note);
  return h(
    "section",
    {},
    h("h1", {}, "Saved ", h("em", {}, clock(state.durationMs))),
    why ? h("p", { class: "note", id: "saved-note", style: "margin:0 0 14px" }, why) : null,
    linkBox(state.upload),
    h("ul", { class: "files", "aria-label": "Saved files" }, ...state.files.map((file: SavedFile) => h("li", {}, h("span", { class: "file", title: file.name }, file.name), h("span", { class: "size" }, bytesText(file.bytes))))),
    retry,
    again,
    h("p", { class: "note" }, "In your Downloads folder, in a Zeno folder."),
  );
}

function failedScreen(state: Extract<RecorderState, { phase: "failed" }>): HTMLElement {
  const again = h("button", { class: "secondary", id: "retry", type: "button" }, "Try again");
  again.addEventListener("click", () => void send({ type: "recorder.dismiss" }));
  return h("section", {}, h("h1", {}, "That didn't ", h("em", {}, "work.")), h("p", { class: "problem", role: "alert" }, state.message), again);
}

/** Everything on screen except the running numbers. */
function keyOf(state: RecorderState): string {
  const upload = "upload" in state ? state.upload : undefined;
  const moving = state.phase === "recording" ? [state.pausedAt !== null, state.pausedMs] : state.phase === "counting" ? [state.remaining] : [];
  return [state.phase, "id" in state ? state.id : "", upload?.phase, upload?.reason, upload?.url, upload?.copied, upload?.message, ...moving].join("|");
}

/** `again` draws it afresh even when nothing about the state changed (the list of unfinished recordings changed). */
function render(state: RecorderState, again = false): void {
  const key = keyOf(state);
  if (again) shownKey = "";
  // Only the numbers of an upload moved: say them in place, so a link that is being selected is not thrown away.
  const line = document.getElementById("upload-line");
  if (key === shownKey && line && "upload" in state) {
    line.textContent = uploadText(state.upload);
    current = state;
    return;
  }
  shownKey = key;
  current = state;
  if (timer) clearInterval(timer);
  timer = null;
  void (async () => {
    const view = await (async (): Promise<HTMLElement> => {
      switch (state.phase) {
        case "idle":
          return idleScreen();
        case "choosing":
          return message(h("span", {}, "Choose what ", h("em", {}, "to share.")), "Pick the screen, window or tab in the browser's window. Nothing is recorded until you do.");
        case "starting":
          return message("Starting…", "One moment.");
        case "counting":
          return countingScreen(state);
        case "recording":
          return recordingScreen(state);
        case "stopping":
          return message("Saving…", "Finishing the files.");
        case "recovering":
          return message("Finishing…", "Closing the recording off, and sending what Zeno does not have.");
        case "saved":
          return savedScreen(state);
        case "failed":
          return failedScreen(state);
      }
    })();
    // A newer state may have arrived while this one was being drawn: draw only the latest.
    if (current === state) app.replaceChildren(view);
  })();
}

chrome.storage.onChanged.addListener((changes, area) => {
  if (area === "session" && changes[STATE_KEY]) render((changes[STATE_KEY].newValue as RecorderState | undefined) ?? IDLE);
});

void (async () => {
  await loadSettings();
  render(await send({ type: "recorder.status" }));
})();
