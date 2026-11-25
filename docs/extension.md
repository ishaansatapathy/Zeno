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
  recording was interrupted, and the start screen offers to finish it (see *Finishing what was cut short*).
* **Pause and resume.** *Pause* holds the recording: nothing is written and the file has no gap, it simply
  goes on when resumed, so a recording that was 8.5 seconds on the clock with a 3-second pause is 5.3
  seconds long (measured: the manifest says 5454 ms, the file 5.33 s). The clock in the popup stands
  still while paused and the toolbar icon says `II` instead of `REC`. The state changes when the
  recorder says it has paused or resumed, not when the button was pressed, so the button waits and a repeated
  or late message cannot pause twice. What was recorded is flushed before pausing. A paused recording still
  holds the screen and the camera, so one left paused for **30 minutes** is stopped and saved (the popup says
  why); the browser tests build with 8 seconds to watch it happen.
* **A count before it begins.** *Countdown* (on by default, remembered) counts 3, 2, 1 after everything is
  chosen and open (the picker, the microphone, the camera) and before anything is recorded or written. The
  number is drawn big over the page being looked at (the tab the person started from) and on the toolbar
  icon, and the popup shows it with *Cancel*; Esc on the page, or closing the share from the browser's own bar,
  also gives up, and nothing is left behind. When it reaches zero the number is taken off the page and the
  recording begins a quarter of a second later, so it is not in the picture.
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
  The upload and the plain share link use the recording's stable UUID, so losing either creation response
  and retrying returns the same resource instead of leaving an extra upload or live link.
  Which track is uploaded: the screen's, or the camera's when there is no screen; the other track stays on the
  computer for the editor.
* **Signed out, or not allowed to upload** (a student's account): the recording works exactly the same, the
  popup says why there is no link, and offers *Sign in to Zeno*.
* **Stopping**: the stop button, or the browser's own *Stop sharing* bar. The finished files are given to
  the browser to save in `Downloads/Zeno/`, named like `Zeno 2026-11-21 14.05.09 screen.webm`, and a copy
  stays in the extension's own storage, which is what an upload will read from.
* **Finishing what was cut short.** The start screen lists, newest first and at most five, the recordings an
  earlier run left on this computer: ones that were *cut short* (the browser or the recorder went before the
  stop) and ones that were stopped but are *not all in Zeno yet*. *Recover* closes a cut-short recording off
  (its manifest is completed, ending at the last thing that reached the disk), hands the files to the browser
  to save like a stopped recording, and sends what Zeno does not have if an upload was pending. *Finish upload*
  resumes that upload. If sign-in is needed, the recording stays on the unfinished list. It is
  the same recording, so it is the same link: the one made when it started, already in somebody's chat, now
  works. *Delete* removes a recording from this computer after asking twice. A saved screen whose upload gave up
  has *Try the upload again*. A finished recording can be tidied away after a week once Chrome confirms that
  every track was downloaded and still exists. If a local save cannot be confirmed, *Save locally* retries it
  from the copy the extension kept.

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
recordings/<id>/manifest.json   status "recording" or "complete", the tracks, when each really began, the pauses
recordings/<id>/upload.json     how the upload stands: pending, failed, done or skipped (and its part size)
recordings/<id>/downloads.json  the browser's download id for each file saved to Downloads/Zeno
```

The manifest records when each track started, because the editor lines the tracks up by it, and every pause
(in wall-clock time while it is recording; once it is stopped also as `joints`: where in the file each pause
happened and how long it lasted, which the file itself does not say). A failed
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

**Pause arithmetic** (`src/shared/pauses.ts`) is the one place that says how long a recording is: the clock in
the popup, the length of a saved recording and the manifest cannot disagree, whether the recording ended
normally or was found cut short. **Recovery decisions** (`src/shared/recovery.ts`) are the same kind of plain
functions over what is on the disk: from the manifest, `upload.json` and the size of the file they say
whether a recording is live (never touched), cut short, waiting for its upload, done with (and when it may be
tidied away), empty or not understood (left alone). `library.ts` is the only part that reads or writes those
files.

**The uploader** (`src/offscreen/uploader.ts`) is plain logic over four things it is given (the API, the
disk, a way to send a part, a way to wait), so it is tested without a browser: which part is next
(`nextPart`, with a test that every byte is covered exactly once for any size), a part that fails is
tried again and never skipped or doubled, a person who is signed out is not uploaded for, an account whose storage is full is told so (`413`, said in words; what is held is kept and the rest goes up once room is made), and it gives up
only some time after the recording has ended. Picking an upload up again asks Zeno what storage holds
(`GET /v1/videos/{id}/upload`), carries on after the parts it holds without a gap (a part past a gap is sent
again: sending twice is harmless, skipping one would leave a hole), and treats an upload that is no longer
open as one that was assembled, which `complete` (asked twice without harm) confirms or refuses. The
recording's id is also its video's id, which is what makes all of it repeatable. The disk is read through the same worker that writes it,
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

* `npm test`: the state machine (counting, pausing, recovering), pause arithmetic, the recovery decisions,
  the badge, the formatting helpers, the uploader (including picking an upload up again) and the pinned
  extension id (108 tests in all).
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
* `e2e/controls.spec.ts` (no API needed): pausing (the clock stands still, the badge changes, and the file,
  read back with `ffprobe`, is as long as it was recorded and has no gap), a repeated or stray pause or resume
  changing nothing, a recording left paused being stopped and saved, the count (cancelling leaves nothing on
  disk; counting through starts the recording after it; drawn on a page and cancelled with Esc), and a
  recording that was killed being offered, recovered into a real video and not offered again, and deleted only
  on the second press. `e2e/recover.spec.ts` (real API and storage): a recording killed after its first 5 MiB
  part went up is recovered under the same link and Zeno ends up with exactly the bytes on the computer, one
  recording and not two; an upload that did not finish is finished without saving the files again; a saved
  screen's failed upload is tried again. The browser tests are built with `npm run build:e2e`, which is the
  ordinary build with the pause limit shortened.
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
  tests with a fake network, not by cutting a real one; a recording whose recorder was killed is finished and
  uploaded (tested), but a whole browser being killed or the machine losing power has not been tried, only
  what the files on disk would look like. The popup's *Copy link* button needs a click in a real browser; the
  clipboard at stop is tested.
* **The count on a real page.** It is drawn through `activeTab`, like the bubble, so it appears only on the tab
  the extension was opened on; the browser tests draw it on the API's own address, which the extension may
  reach, not through a toolbar click. On a page the extension may not draw on (the browser's own pages, the web
  store) there is no number on the page, only on the icon and in the popup.
* **A seam, not a join.** Chrome writes a paused recording without a gap (checked: 5.33 s for 5.45 s
  recorded), but the camera and the screen are paused one after the other, a few milliseconds apart, and
  their tracks are lined up by the time each began; the editor will have to place the pauses (`joints`) per
  track if that matters to it.
* **Re-sent, not resumed, within a part.** Picking an upload up again skips the whole parts Zeno holds; the
  part that was in flight when the browser went is sent again from its start (at most 5 MiB), and so is the
  last, shorter part, which Zeno does not list as held.
* **Trim is not here.** Trimming the start and end of a recording was planned for this day and is not done:
  the honest version removes the footage (a re-encode in the worker, after which the link plays the shorter
  video), because a trim that only hides the start still leaves it in the segments anyone with the link can
  fetch. It needs a worker job and an API route, and is its own piece of work.
* **CPU.** VP9 is encoded in software. Fine for a camera or a 1080p screen on a modern machine; a 4K
  screen at 30 frames a second may not be. Hardware H.264 is the alternative if it shows.
* **Chrome 116 or later.** Branded Chrome 137 and later no longer loads unpacked extensions from the
  command line, so the tests use Chromium; people load the built extension from `chrome://extensions` as above.
