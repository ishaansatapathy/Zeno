# Web app reference

How the Next.js app in `web/` talks to the API, what it guarantees on the account screens, and how
it is tested. The marketing site and the signed-in app are one Next.js project.

## Talking to the API

The browser calls `/v1/...` on **the page's own origin**. In development Next forwards that path to
the API (`rewrites` in `web/next.config.mjs`, `API_URL`, default `http://127.0.0.1:8080`); in
production the edge routes `/v1` to the API and Next is not involved. Because the origin is the
page's own:

* the session cookies belong to the page. The API sets them, the browser attaches them, and no
  JavaScript can read them (`HttpOnly`, `SameSite=Lax`). There is nothing to store and nothing to leak;
* there is no CORS to configure, and the `Origin` the API checks on every state-changing request is
  the page's own;
* the refresh cookie's path (`/v1/auth`) means the same thing in both places.

Behind the development proxy the API sees the web server's address, not the visitor's. In
production the real proxy and `TRUSTED_PROXY_HOPS` decide that, as for any other client.

### The typed client

`web/lib/api/client.ts` is the only code that calls the API. Its types are generated from
[`docs/openapi.json`](openapi.json) (`npm run api:types`), so a route, field or status the API no
longer has is a type error in the web app. CI runs `npm run api:check`, which fails if the
generated files are not what the document says.

Below the call sites it does two things no screen has to remember:

* **Renews the session.** An access token lasts minutes. A request that answers
  `401 unauthenticated` renews the session once (`POST /v1/auth/refresh`) and is repeated. Many
  requests failing together renew once, and so do many tabs: the renewal runs under a Web Lock,
  because a refresh token works only once. A tab that loses a race (`refresh_conflict`) repeats its
  request instead, since the other tab's new cookies are already its own. A session that cannot be
  renewed returns the original `401`, and the app sends the person to sign in.
* **Returns every failure in one shape**, `{status, code, message, fields, retryAfter}`, including a
  server that cannot be reached (`status: 0, code: "network"`). A screen branches on `code`, shows
  `fields` against their inputs, and turns `retryAfter` into "try again in 2 minutes".

Forms check what a form can know before asking the server (`components/auth/validate.ts`, with the
limits taken from the same document, so they cannot drift from the API), and show the server's
objections beside the field they are about. The server stays the authority.

## Account screens

| Route | Purpose |
| --- | --- |
| `/register`, `/login`, `/forgot-password` | The forms. `?next=` on `/login` is honoured only for a path on this site (`lib/next-path.ts`) |
| `/verify-email`, `/verify-email/:token` | Ask for a new link; open the emailed link |
| `/reset-password/:token` | Choose a new password |
| `/library` | The signed-in app: the person's recordings. Nothing is shown before the API has said who is signed in; a signed-out visitor is sent to `/login?next=...` and brought back |

The paths are the ones the API puts in its emails (`/verify-email/:token`, `/reset-password/:token`,
`/login`).

* **A link in an email does nothing until it is asked to.** Mail scanners and link previews open
  links to look at them; a page that confirmed an address on being opened would have used the
  token up before the person arrived. The page shows a button instead.
* **Pages with a token in the path** are served with `Referrer-Policy: no-referrer` and
  `Cache-Control: no-store`, so the token does not leave in a `Referer` or stay in a cache.
* **Every account screen is `noindex`.**
* **Answers that must not reveal who has an account** (sign-up, forgot password, resend) are shown as
  "if this address can be used, a link is on its way".
* **A failed fetch is not a sign-out.** If the API cannot be reached the app says so and offers to
  try again; it does not send the person to the sign-in page.

## The library

`/library` lists the signed-in person's recordings, newest first, twenty at a time (`Load more`
follows the API's keyset cursor, so a page costs the same however far back it goes and nothing is
skipped or repeated if recordings arrive meanwhile). It can be narrowed to how a recording was made
(upload, extension, studio, stream, meeting). Each row shows its title, source, date, length, status,
who may watch it and its views, and offers three things:

* **Rename**, in place. The request carries the version the row was read at. If the recording was
  changed elsewhere (another tab, another device) the API answers `409 version_conflict`; the row is
  reloaded to what it now is and the person is told, instead of their rename silently overwriting the
  other one. Enter saves, Escape cancels, and focus goes back to the button that opened the editor.
* **Delete**, after an inline question that defaults to keeping it ("Keep it" has the focus) and says
  what goes with it: the video, and every link. A recording already deleted elsewhere counts as deleted.
* **Copy link**. It reuses the recording's existing link when there is one that works for anyone (no
  password, no end date), and makes one only when there is not, so pressing it twice does not pile up
  links. Making the first link switches a private recording to "anyone with the link", which is an edit,
  so the row takes the recording's new version straight away or the next rename would be refused as
  out of date. If the browser refuses to write to the clipboard, the address is shown in a box to copy
  by hand. The address points at the share page, which is built later: until then it does not open.

An account that only watches (a student) has no library: the API answers `403 forbidden`, and the page
says so in words instead of showing an error. The list's logic (merging pages, replacing and removing
rows, choosing the link to hand out, wording) is plain functions in `components/library/rows.ts`,
unit-tested apart from React.

## The share page

`/s/<code>` is what a share link opens: public, no account, no cookie. It is the page that will be
opened most, by people who have never heard of Zeno, so it is built to cost the API almost nothing.

**One API request per link per 15 seconds.** The server renders the page (`app/s/[code]/page.tsx`)
with `revalidate = 15` and fetches the link's state with the same 15 seconds (`lib/api/server.ts`),
so however many people open a link, the API hears about it about four times a minute, and an edge in
front of Next can keep the page for the same time (`Cache-Control: s-maxage=15, stale-while-revalidate`
is what Next sends). The page, its metadata and its preview image ask for the link in one render and
share one fetch. A test holds this to account: 25 page views of one link are at most one request in the
API's log (it only runs against `next start`, since `next dev` renders every request). An API that
cannot be reached is an error, not an answer, so Next keeps serving the last good page rather than
caching one that says the link is gone.

**What it says.** A code of the wrong shape is a 404 without any call. Otherwise, from what the API said:
a video that is ready shows a cover with a play button; one that is not (being recorded, uploading,
processing) says so and asks again by itself, after 2 seconds, then 3, 5, 7 ... slowing to 30 (a video is
usually playable seconds after the recording stops, and somebody who opens the link then is waiting for exactly
that; the API keeps such an answer for 2 seconds, not the usual 15, so asking often costs the origin almost
nothing), and not while the tab is hidden; a failed one says so; a closed link says it is no longer available. A link with a password shows
a password box and nothing else: not the title, not in the page, not in the preview.
A page reused after a revoke shows a player that is refused when played, never a video that plays,
because playing asks the database again. (A closed link is a 200 page that says so, not a 410: Next
cannot choose the status of a page it caches.)

**The player** (`components/share/Player.tsx`, with its rules in `state.ts`). Pressing play starts a
viewing (`POST /v1/s/:code/play`) and hands the playlist to Safari's own HLS, or to hls.js, which is
only downloaded at that moment, so a page that nobody plays costs no player code. Playback carries a
watermark that moves every 25 seconds. If the server refuses anything on the way (the link closed, the
viewing ran out, a key was refused) the viewing ends at the first refusal, because hls.js reports each
refusal as a non-fatal error and keeps asking; the page says "This viewing has ended" and offers to
start again. Every other way playing can fail (a wrong password, a limit, the server failing, a browser
that cannot play HLS) is a sentence, never a dead end.

**Link previews.** The page sets a title, description and `og:` and `twitter:` tags, and
`opengraph-image.tsx` draws a card (the wordmark, the title in the site's serif; a protected or closed
link's says nothing about the video). `SITE_URL` makes the image address absolute. The page is
`noindex` and sent with `Referrer-Policy: no-referrer`: the address is a capability.

**Settings** (`/settings`, behind sign-in; the person's name in the bar leads to it): how much storage is used
(a meter with its numbers in words for a screen reader, any use at all showing as at least 1%, the way out
when it is full), the name (kept, and shown in the bar at once), and a new password (the old one asked for;
it signs the person out everywhere and sends them to sign in, with a note saying why). Checked in a browser
(`e2e/settings.spec.ts`, 7 tests, axe at desktop and phone size with and without problems on screen). The
full and unavailable states are tested by answering the storage request with a made-up response, since a
test cannot give an account a smaller allowance through the API.

**Real playback, and why the browser tests do not cover the picture.** Playwright's Chromium cannot
decode H.264, so the suite checks the whole way up to the segment (the page, the viewing, the playlists,
the watermark, every way the server can refuse) against recorded answers. The picture itself is checked
by `npm run check:playback -- --api-log <file>` (`scripts/real-playback.mjs`), which needs the whole stack
running and an installed Chrome: it makes a video with ffmpeg, uploads it through the API, lets the real
worker encode it, opens its share link in Chrome, presses play, and then revokes the link while it plays.
What it found, on a laptop: a 60 second 640x360 clip was ready 9 seconds after upload; the first frame
was on screen 2.6 seconds after pressing play, at the full 640x360 (hls.js is told to assume a decent
connection and to fetch no more picture than the player can show, because by default it started at
144p and, at the speed a clip downloads, the whole clip arrived in 144p); the watermark showed; and a
link revoked mid-play ends the viewing within about half a minute (24.5, 10.2 and 27.1 seconds in three
runs with only the keys acting, the player's own link check shut out with `--keys-only`; see *Revoking a
viewing* in backend.md). Before keys changed as the video plays, a viewer who had the key played the whole
video to the end.
Recent Chrome plays HLS by itself (`canPlayType` says "maybe"), but its own player failed on our key
flow, so hls.js is used wherever it runs, and the browser's own HLS only where it does not (an iPhone
before iOS 17.1). The cause of Chrome's failure was not chased down.

**Comments.** Under the player, `Comments` (components/share/Comments.tsx) shows what was said, newest first
(`Older comments` for the next page, and the first page asked for again every 20 seconds while the tab is
visible). Anyone with the link reads. Leaving one needs an account: a person who is not signed in who
presses *Comment* is told so with a link to sign in that comes back to this page, and what they typed is
kept. A comment is pinned to the moment the video is at (a box to switch that off); six reaction buttons
leave an emoji at the current moment and wait for the video to be playing, since without it there is no
moment. A bar under the heading marks the moments people reacted to (grouped, so a hundred reactions in
the same minute are one mark with a number), and pressing a mark or a time in the list goes there. Each
comment has *Remove* if it is yours or the recording is yours. Words are drawn as text, never as markup.
Posting uses a client-made id, so trying again after a dropped connection posts once. For a link with a
password the panel appears when the viewing starts, with that viewing's token. The owner switches the
conversation off and on in the library (*Comments on/off* on each row); off, visitors see no panel.
The logic (merging pages, marks, the words for people) is plain functions in `conversation.ts` with
tests; the screen is tested in `e2e/comments.spec.ts`. Not tested end to end: seeking, and pinning to the playhead (the browser tests cannot decode H.264, so they
never have a playing video); `scripts/real-playback.mjs` is where that is tried against real Chrome.

**A viewing keeps checking its link.** Every 30 seconds a viewing asks `GET /v1/s/:code` (cached by the
API and the edge for 15 seconds); a `404` or `410` turns the page into the closed notice.

## Tests

* `npm test`: unit tests (the client's renewal and error mapping against a fake server, the
  redirect guard, validation, wording).
* `npm run e2e`: Playwright against the real API, Postgres, Redis and object storage
  (`docker compose -f deploy/docker-compose.yml up -d --wait` first). The API runs with
  `LOG_FORMAT=json` writing to `web/e2e/.logs/api.log`, because the development mail transport prints
  each email there and the tests read the link from it. It uses Redis database 15, emptied before the
  run, so earlier runs' rate limits do not count. An API already running on port 8080 has to be
  stopped first. The suite covers register, confirm, sign out and in, wrong and unconfirmed sign-ins,
  forgot and reset (and a reset link working once), the sign-in redirect and the open-redirect guard,
  silent session renewal, a session that cannot be renewed, the token-page headers, and axe-core
  checks of every account screen at desktop and phone sizes. For the library: an empty library, a new
  recording and what the row says, rename (saved, cancelled, refused, and the conflict case), delete
  (kept, then deleted for good), copy link (made once, reused, replaced after a revoke, and rename
  still working afterwards), 23 recordings read in two pages in order, the source filter, the
  student's message, and axe-core on the library in every state a row can be in. Signing up is limited
  to ten a hour from one address, so tests that only need somebody signed in share one account
  (`sharedAccount` in `e2e/helpers.ts`); the library tests empty its library before each test.
