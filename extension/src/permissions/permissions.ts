// The browser asks for the camera and the microphone with a prompt, and a popup cannot show one: it
// closes the moment the prompt takes the focus. A page can. This is that page. It asks once, remembers
// nothing, and closes itself.

const query = new URLSearchParams(location.search);
const wantsMicrophone = query.get("microphone") === "true";
const wantsCamera = query.get("camera") === "true";
const message = document.getElementById("message") as HTMLElement;
const button = document.getElementById("allow") as HTMLButtonElement;

button.addEventListener("click", async () => {
  button.disabled = true;
  try {
    const stream = await navigator.mediaDevices.getUserMedia({ audio: wantsMicrophone, video: wantsCamera });
    // The permission is the point. Nothing is recorded: let go of the devices at once.
    stream.getTracks().forEach((track) => track.stop());
    message.textContent = "Done. Open Zeno again and press Start recording.";
    message.classList.remove("problem");
    button.hidden = true;
    setTimeout(() => window.close(), 1500);
  } catch (error) {
    const name = (error as { name?: string }).name;
    message.classList.add("problem");
    message.textContent =
      name === "NotAllowedError"
        ? "It was not allowed. Click the camera icon in the address bar to allow it, then try again."
        : "No camera or microphone could be opened. Is another app using it?";
    button.disabled = false;
  }
});
