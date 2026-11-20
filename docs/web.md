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
| `/library` | The signed-in app. Nothing is shown before the API has said who is signed in; a signed-out visitor is sent to `/login?next=...` and brought back |

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
  checks of every account screen at desktop and phone sizes.
