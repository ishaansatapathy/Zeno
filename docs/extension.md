# Browser extension reference

The Chrome extension in `extension/` (Manifest V3, TypeScript). It records the screen, the camera and the
microphone, and keeps what it records as separate files that survive a crash. Today it saves them to the
person's Downloads folder; uploading while recording and the share link are the next steps (see
`DAILY_PLAN.md`, Days 25 and 26). It follows the rule in [studio-design.md](studio-design.md): the work is
done on the person's machine, and nothing goes to our servers until they share.

## Try it

```
cd extension && npm ci && npm run build
```

Then open `chrome://extensions`, switch on Developer mode, choose *Load unpacked* and pick
`extension/dist`. Pin the Zeno icon and click it.

## What it does now

* **A popup** to choose what to record: the screen (a tab, a window or everything), the sound of the
  screen, the microphone, the camera, and which microphone and camera when there are several. The choices
  are remembered. Closing the popup does not stop a recording; reopening it shows the time and the stop
  button.
* **Separate tracks, not one flattened picture.** The screen (with the microphone's and the screen's
  sound mixed into it) and the camera are recorded to their own files. A picture that has been combined
  cannot be edited: the face cannot be moved out of the way of what is happening, or reframed for a
  vertical clip. The editor will put the tracks together.
* **Crash-safe.** Every second a chunk is written to disk and flushed (see below). Kill the browser or the
  machine and at most the last second is lost, and what was written is a video. The popup then says the
  recording was interrupted.
* **A camera bubble** drawn on the page being shared, so the face is in the recording of that page; it is
  draggable and double-clicking it makes it larger or smaller.
* **Stopping**: the stop button, or the browser's own *Stop sharing* bar. The finished files are given to
  the browser to save in `Downloads/Zeno/`, named like `Zeno 2026-11-21 14.05.09 screen.webm`, and a copy
  stays in the extension's own storage, which is what an upload will read from.

## How it is built

Four parts, and every message between them is typed in one file (`src/shared/messages.ts`):

| Part | Job | Why it is separate |
| --- | --- | --- |
| Popup | Shows the state, sends the person's choices | A popup closes whenever it loses focus, so it can hold nothing |
| Service worker | Owns the recorder's state; tells the others what to do | The browser stops it whenever it is idle, so it keeps nothing in memory: the state is in `chrome.storage.session`, every change goes through a pure `reduce`, and changes are applied one at a time |
| Offscreen document | Holds the streams and the recorders | A service worker cannot hold a stream; a visible page would be closed by accident. An offscreen document is invisible and is kept alive while it uses media |
| Camera bubble | Shows the camera on the shared page | It is a page of the extension's own, in a frame, so the camera permission the extension already has applies instead of asking again on every site |

**State** (`src/shared/state.ts`) is one pure function with a test for every transition. A message that
makes no sense in the current state (a second *start*, a late *stopped*, a *cancel* after recording has
begun) changes nothing, so a repeated or late message cannot corrupt a recording. If the state says a
recording is under way and the offscreen document is gone, the next look at it says so instead of showing a
recorder that is not there.

**Disk.** `MediaRecorder` hands over a chunk a second. Each is written by a dedicated worker
(`writer.worker.ts`) through the browser's private file system, using the synchronous access handle and
flushing after every chunk: that is the only way OPFS offers a flushed write, and a recording written in
one go at the end, or held in memory, is lost with the browser. Layout:

```
recordings/<id>/screen.webm
recordings/<id>/camera.webm
recordings/<id>/manifest.json   status "recording" or "complete", the tracks, when each really began
```

The manifest records when each track started, because the editor lines the tracks up by it. A failed
write (a full disk) is remembered and reported at the next chunk and at stop; what was recorded so far is
kept.

**Screen.** The offscreen document asks the browser for the screen with `getDisplayMedia` (the reason
`DISPLAY_MEDIA`), so the picker and its *Share audio* checkbox are the browser's own. This needs neither
the `desktopCapture` nor the `tabs` permission. Closing the picker without choosing is not an error: the
popup goes quietly back to the start.

**Permissions.** `offscreen`, `downloads`, `storage`, `unlimitedStorage` (a recording is large),
`activeTab` and `scripting` (to draw the bubble on the tab the person is looking at, which the browser
allows because they just opened the extension on it). No access to any site is requested. The browser asks
for the camera and microphone with a prompt that a popup cannot show, so a small page
(`permissions.html`) asks once and closes itself.

## Tests

* `npm test`: the state machine (23 tests with the formatting helpers).
* `npm run e2e`: builds, then loads the extension into Chromium with the browser's fake camera and
  microphone (a moving test picture and a tone). It records through the real popup, reads the files back out
  of the extension's storage, and has `ffprobe` check that they are video: a VP9 or VP8 picture, an Opus
  voice, the right length. It also kills the recorder in the middle and checks that what was written is still a
  video and that the popup says so; closes and reopens the popup mid-recording; refuses a second start; and
  checks the bubble shows the camera. Needs `ffprobe`.
* `ZENO_SCREEN=1 npm run e2e`: also records a real screen, in a visible window that picks the screen for
  itself. It needs a display, so it is not part of CI.

## Not yet verified, and limits

* **The picker, for a real person.** The tests use the browser's fake prompt, which chooses the screen for
  them. That the real picker appears from the offscreen document, and that *Share audio* gives the screen's
  sound, needs a person to try it (and is the first thing to try on a Mac, where Chrome shares tab audio
  but not system audio).
* **The bubble on a real page.** It is drawn through `activeTab`, which the browser grants when the
  extension is opened from its toolbar icon. A test cannot do that, so the bubble is tested on its own page
  only.
* **A recorder's WebM has no length in its header** (and no index for seeking), because it is written as
  it is made. `ffprobe` says "N/A". The worker already treats a missing duration as unknown rather than an
  error, but a real recording has to go through it to be sure (Day 25); a remux (`-c copy`) fixes the
  header if it is needed.
* **CPU.** VP9 is encoded in software. Fine for a camera or a 1080p screen on a modern machine; a 4K
  screen at 30 frames a second may not be. Hardware H.264 is the alternative if it shows.
* **Chrome 116 or later.** Branded Chrome 137 and later no longer loads unpacked extensions from the
  command line, so the tests use Chromium; people load the built extension from `chrome://extensions` as above.
