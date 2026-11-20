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
