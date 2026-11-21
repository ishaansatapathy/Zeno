// Puts the camera bubble on the page being shared. Runs in the page, and only when the service worker
// asks for it (after a recording that includes the screen and the camera has started).

(() => {
  const ID = "zeno-camera-bubble";
  if (document.getElementById(ID)) return;

  const SMALL = 200;
  const LARGE = 320;
  const frame = document.createElement("iframe");
  frame.id = ID;
  frame.src = chrome.runtime.getURL("bubble.html");
  frame.allow = "camera";
  frame.title = "Zeno camera";
  let size = SMALL;
  let left = 24;
  let top = window.innerHeight - SMALL - 24;
  Object.assign(frame.style, {
    all: "initial",
    position: "fixed",
    zIndex: "2147483647",
    border: "0",
    borderRadius: "50%",
    boxShadow: "0 8px 32px rgba(0,0,0,0.45)",
    background: "#000",
    colorScheme: "normal",
  });

  const place = () => {
    // Never off the edge of the page, so it cannot be dragged out of reach.
    left = Math.min(Math.max(0, left), Math.max(0, window.innerWidth - size));
    top = Math.min(Math.max(0, top), Math.max(0, window.innerHeight - size));
    Object.assign(frame.style, { left: `${left}px`, top: `${top}px`, width: `${size}px`, height: `${size}px` });
  };

  window.addEventListener("message", (event: MessageEvent) => {
    // Only the bubble's own frame may move it: a page cannot steer it by posting messages.
    if (event.source !== frame.contentWindow) return;
    const message = event.data as { zenoBubble?: string; dx?: number; dy?: number } | null;
    if (message?.zenoBubble === "move" && typeof message.dx === "number" && typeof message.dy === "number") {
      left += message.dx;
      top += message.dy;
    } else if (message?.zenoBubble === "size") {
      size = size === SMALL ? LARGE : SMALL;
    } else {
      return;
    }
    place();
  });
  window.addEventListener("resize", place);

  place();
  document.documentElement.append(frame);
})();
