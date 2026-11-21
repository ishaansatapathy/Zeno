// The camera bubble: the person's face, drawn on the page that is being shared, so it is in the recording
// of that page. It lives in a frame (a page of the extension's own, so the camera permission already
// granted to the extension applies, rather than asking again on every site), and tells the page that
// holds it when it is dragged.

const video = document.getElementById("camera") as HTMLVideoElement;

void (async () => {
  const stored = await chrome.storage.local.get("settings");
  const id = (stored["settings"] as { cameraId?: string } | undefined)?.cameraId;
  try {
    video.srcObject = await navigator.mediaDevices.getUserMedia({ video: { deviceId: id ? { exact: id } : undefined, width: { ideal: 640 }, height: { ideal: 640 } } });
  } catch {
    const note = document.createElement("p");
    note.textContent = "Camera unavailable";
    document.body.replaceChildren(note);
  }
})();

let last: { x: number; y: number } | null = null;
const tell = (message: { zenoBubble: "move"; dx: number; dy: number } | { zenoBubble: "size" }) => window.parent.postMessage(message, "*");

addEventListener("pointerdown", (event) => {
  last = { x: event.screenX, y: event.screenY };
  document.documentElement.style.cursor = "grabbing";
});
addEventListener("pointermove", (event) => {
  if (!last) return;
  // Screen coordinates, because the frame moves under the pointer and its own would never settle.
  tell({ zenoBubble: "move", dx: event.screenX - last.x, dy: event.screenY - last.y });
  last = { x: event.screenX, y: event.screenY };
});
const drop = () => {
  last = null;
  document.documentElement.style.cursor = "grab";
};
addEventListener("pointerup", drop);
addEventListener("pointercancel", drop);
addEventListener("dblclick", () => tell({ zenoBubble: "size" }));
