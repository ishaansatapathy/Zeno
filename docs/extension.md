# Browser extension reference

The Chrome extension in `extension/` (Manifest V3, TypeScript). It records the screen, the camera and the
microphone, keeps what it records as separate files that survive a crash, and, for a person who is signed
in to Zeno, uploads the recording while it is being made and has the share link on the clipboard the
moment they stop. It follows the rule in [studio-design.md](studio-design.md): the work is done on the
person's machine; the upload goes straight to storage, never through our servers, and the recording is
saved on the machine whatever the network does.

## Try it

```
cd extension && npm ci && npm run build
```

Then open `chrome://extensions`, switch on Developer mode, choose *Load unpacked* and pick
`extension/dist`. Pin the Zeno icon and click it. To get a share link, be signed in to Zeno in the same
browser (the extension has no password of its own: it uses the web app's session).

The extension is built for one API address (default `http://localhost:8080`) and one web address
(`http://localhost:3000`); the browser is asked for access to that API address and no other site:

```
ZENO_API_URL=https://api.example.com ZENO_WEB_URL=https://example.com npm run build
```

Its id is fixed by the `key` in `src/manifest.json` (`fbfpfadnhlgdhefbdmphfdikmpkjmade` for development),
so the API (`EXTENSION_ORIGINS`) and storage (CORS) can be told to trust it; a test keeps the id, the
compose file, `.env.example` and CI in step. The Web Store will give the published extension its own id,
which is then added to the same two lists.

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
* **A share link before the recording ends.** For a person signed in to Zeno, the link is made at the *start*:
  the upload, the recording it belongs to and its link are set up while the first seconds are recorded. The
  link is shown (and can be copied) during the recording, the page behind it shows the video as still being processed until the
  video is ready, and at stop it is put on the clipboard. Stopping to link took 176 to 184 ms in the
  end-to-end test, because nothing has to be waited for: only what is left of the recording (less than one
  5 MiB part) goes up afterwards.
* **Upload while recording.** The file on disk is the truth and the upload follows it: each 5 MiB of it is
  read back and sent straight to storage, using an address the API signs for exactly that length. If the
  network is slow, down or flaky, parts wait on disk and catch up (every try asks for a fresh address, and
  waits 0.5 s, 1 s, 2 s ... up to 15 s), and the recording itself is never held up. After the recording ends
  it keeps trying for 90 seconds, then says it could not finish and that the copy on the computer is kept.
  Which track is uploaded: the screen's, or the camera's when there is no screen; the other track stays on the
  computer for the editor.
* **Signed out, or not allowed to upload** (a student's account): the recording works exactly the same, the
  popup says why there is no link, and offers *Sign in to Zeno*.
* **Stopping**: the stop button, or the browser's own *Stop sharing* bar. The finished files are given to
  the browser to save in `Downloads/Zeno/`, named like `Zeno 2026-11-21 14.05.09 screen.webm`, and a copy
  stays in the extension's own storage, which is what an upload will read from.

## How it is built

Four parts, and every message between them is typed in one file (`src/shared/messages.ts`). The
service worker also asks the API who is signed in; the offscreen document does the uploading:

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

**The API.** `src/shared/api.ts` is a typed client (`openapi-fetch`) over the types generated from
`docs/openapi.json` (`npm run api:types`, checked by `npm run api:check` in CI), so a route, a field or
a status the API no longer has is a type error in the extension. Calls carry the person's own session cookie
(the browser sends it because the extension may reach the API's address); a call that answers `401` renews
the session once and repeats, and a server that cannot be reached comes back in the same shape as any
failure.

**The uploader** (`src/offscreen/uploader.ts`) is plain logic over four things it is given (the API, the
disk, a way to send a part, a way to wait), so it is tested without a browser: which part is next
(`nextPart`, with a test that every byte is covered exactly once for any size), a part that fails is
tried again and never skipped or doubled, a person who is signed out is not uploaded for, and it gives up
only some time after the recording has ended. The disk is read through the same worker that writes it,
queued behind the writes, so it never sees half a chunk and never reaches a file that is being closed.

**Permissions.** `offscreen`, `downloads`, `storage`, `unlimitedStorage` (a recording is large),
`activeTab` and `scripting` (to draw the bubble on the tab the person is looking at, which the browser
allows because they just opened the extension on it), and `clipboardWrite` (the link is copied from the
offscreen document, which may, when the popup has already closed). One site is requested: the API's own
address, so the browser sends the session and does not ask the API CORS questions for the extension.
Storage is not requested; it allows the extension's origin in its own CORS rule instead. The browser asks
for the camera and microphone with a prompt that a popup cannot show, so a small page
(`permissions.html`) asks once and closes itself.

## Tests

* `npm test`: the state machine, the formatting helpers, the uploader (45 tests in all) and the pinned
  extension id.
* `npm run e2e`: builds, then loads the extension into Chromium with the browser's fake camera and
  microphone (a moving test picture and a tone). It records through the real popup, reads the files back out
  of the extension's storage, and has `ffprobe` check that they are video: a VP9 or VP8 picture, an Opus
  voice, the right length. It also kills the recorder in the middle and checks that what was written is still a
  video and that the popup says so; closes and reopens the popup mid-recording; refuses a second start; and
  checks the bubble shows the camera. Needs `ffprobe`.
  `e2e/upload.spec.ts` runs against the real API and storage (the Playwright config starts the API;
  `docker compose -f deploy/docker-compose.yml up -d` provides Postgres, Redis and MinIO): signed out, the
  recording works and the popup says what a link needs; signed in, the link is there while recording,
  parts go up before stop, the link is on the clipboard at stop, and afterwards Zeno holds exactly the
  bytes recorded and the link's page says the video is still processing; and an account that may not upload records
  locally and is told so.
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
  it is made. Checked on Day 25: two recordings made by the extension (about a minute of the fake camera,
  uploaded in parts) went through the real worker and came out ready, 1280x720, six renditions.
* **What has not been tried.** The upload on a slow or interrupted network is covered by the uploader's
  tests with a fake network, not by cutting a real one; a recording whose browser dies mid-way keeps its
  file on disk, but nothing yet finishes its upload at the next start (Day 28). The popup's *Copy link*
  button needs a click in a real browser; the clipboard at stop is tested.
* **CPU.** VP9 is encoded in software. Fine for a camera or a 1080p screen on a modern machine; a 4K
  screen at 30 frames a second may not be. Hardware H.264 is the alternative if it shows.
* **Chrome 116 or later.** Branded Chrome 137 and later no longer loads unpacked extensions from the
  command line, so the tests use Chromium; people load the built extension from `chrome://extensions` as above.
