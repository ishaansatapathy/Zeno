// What the person sees. It holds no recording and decides nothing: it shows the state the service worker
// keeps (and re-shows it whenever that changes, so closing and reopening the popup loses nothing) and
// sends the person's choices as commands.

import { bytesText, clock } from "../shared/format";
import type { Command, RecordingOptions, SavedFile } from "../shared/messages";
import { IDLE, type RecorderState } from "../shared/state";

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
}

const DEFAULTS: Settings = { screen: true, systemAudio: false, microphone: true, camera: false };
let settings: Settings = { ...DEFAULTS };
let timer: ReturnType<typeof setInterval> | null = null;
let current: RecorderState = IDLE;

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

async function send(command: Command): Promise<RecorderState> {
  return (await chrome.runtime.sendMessage(command)) as RecorderState;
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

async function idleScreen(): Promise<HTMLElement> {
  const microphones = settings.microphone ? await devices("audioinput") : [];
  const cameras = settings.camera ? await devices("videoinput") : [];
  const items = [
    option("screen", "Screen", "A tab, a window or the whole screen"),
    option("systemAudio", "Sound from the screen", "Tick Share audio in the next step too", { sub: true, disabled: !settings.screen }),
    option("microphone", "Microphone", "Your voice"),
    option("camera", "Camera", "Shown over the screen and saved as its own file"),
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
    list,
    start,
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

function recordingScreen(state: Extract<RecorderState, { phase: "recording" }>): HTMLElement {
  const time = h("span", { class: "clock", id: "clock" }, clock(Date.now() - state.startedAt));
  const stop = h("button", { class: "primary", id: "stop", type: "button" }, "Stop and save");
  stop.addEventListener("click", () => {
    stop.disabled = true;
    void send({ type: "recorder.stop" });
  });
  timer = setInterval(() => {
    time.textContent = clock(Date.now() - state.startedAt);
  }, 250);
  const names = state.tracks.map((track) => (track === "screen" ? "Screen" : "Camera")).join(" and ");
  return h(
    "section",
    {},
    h("div", { class: "status", role: "status" }, h("span", { class: "dot", "aria-hidden": "true" }), time),
    h("p", { class: "label", style: "margin-bottom:20px" }, `Recording ${names}`),
    stop,
    h("p", { class: "note" }, "You can also stop from the browser's own Stop sharing bar."),
  );
}

function savedScreen(state: Extract<RecorderState, { phase: "saved" }>): HTMLElement {
  const again = h("button", { class: "secondary", id: "again", type: "button" }, "Record again");
  again.addEventListener("click", () => void send({ type: "recorder.dismiss" }));
  return h(
    "section",
    {},
    h("h1", {}, "Saved ", h("em", {}, clock(state.durationMs))),
    h("ul", { class: "files", "aria-label": "Saved files" }, ...state.files.map((file: SavedFile) => h("li", {}, h("span", { class: "file", title: file.name }, file.name), h("span", { class: "size" }, bytesText(file.bytes))))),
    again,
    h("p", { class: "note" }, "In your Downloads folder, in a Zeno folder."),
  );
}

function failedScreen(state: Extract<RecorderState, { phase: "failed" }>): HTMLElement {
  const again = h("button", { class: "secondary", id: "retry", type: "button" }, "Try again");
  again.addEventListener("click", () => void send({ type: "recorder.dismiss" }));
  return h("section", {}, h("h1", {}, "That didn't ", h("em", {}, "work.")), h("p", { class: "problem", role: "alert" }, state.message), again);
}

function render(state: RecorderState): void {
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
        case "recording":
          return recordingScreen(state);
        case "stopping":
          return message("Saving…", "Finishing the files.");
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
