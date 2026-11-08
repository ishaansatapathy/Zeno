# Zeno

Secure video and live-class platform for Indian educators. Full plan: [docs/Zeno_Product_and_Technical_Plan.pdf](docs/Zeno_Product_and_Technical_Plan.pdf).

## Layout

```
crates/
  core/        config, Postgres pool + migrator, shared domain types
  api/         axum HTTP server: auth, courses, videos, playback manifests, webhooks
  worker/      transcode job runner (FFmpeg)
  live/        stream-key check and live HLS control
  keyserver/   token validation and AES-128 key delivery
web/           Next.js creator dashboard and student app
deploy/        docker-compose, SQL migrations
tests/e2e/     Playwright end-to-end suites
```

## Running locally

Needs Rust (rustup), Docker, Node 20+, and ffmpeg (with libx264) on PATH for the worker.

```
cp .env.example .env              # then set JWT_ACCESS_SECRET (openssl rand -base64 48)
docker compose -f deploy/docker-compose.yml up -d --wait   # Postgres :5433, Redis :6379, MinIO :9000
cargo run -p zeno-api             # http://localhost:8080/health  (applies migrations in development)
cargo run -p zeno-worker          # transcodes uploaded videos
cargo run -p zeno-keyserver       # http://localhost:8081, content keys for playback

cd web && npm install && npm run dev   # http://localhost:3000
```

In development emails are not sent; the verification and reset links are printed in the API log.

Tests (the integration suites use a real Postgres, each test in its own throwaway database):

```
export DATABASE_URL=postgres://zeno:zeno@127.0.0.1:5433/zeno
export TEST_REDIS_URL=redis://127.0.0.1:6379
# S3_* come from .env; the worker tests also run real ffmpeg
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

## Auth API

Cookie-based (`HttpOnly`, `SameSite=Lax`): a 15-minute access JWT plus a rotating 30-day refresh token that is only sent to `/v1/auth/*`.

| Endpoint | Purpose |
| --- | --- |
| `POST /v1/auth/register` | Create a student or creator account; always answers `202` so it can't be used to discover who is registered |
| `POST /v1/auth/verify-email` | Spend the emailed token (a POST, so mail scanners that prefetch links can't burn it); signs the user in |
| `POST /v1/auth/resend-verification` | New verification link, capped per address |
| `POST /v1/auth/login` | Sign in; per-account-and-address lockout after 5 failures in 15 minutes |
| `POST /v1/auth/refresh` | Rotate the refresh token; reuse of a rotated token revokes the whole session family |
| `POST /v1/auth/logout` | End this device's session |
| `POST /v1/auth/forgot-password` / `reset-password` | Emailed single-use link; reset signs out every device |
| `GET /v1/auth/me` | The signed-in user |
| `GET /v1/auth/activity` | The signed-in user's last 20 security events (sign-ins, failures, resets) |

Sessions end immediately on logout, password reset and detected token theft: each access token carries its session family, and revoking a family marks it in Redis for the token's remaining lifetime. Anything that differs between "account exists" and "doesn't" (token writes, email) runs after the response, so timing doesn't reveal accounts. Expired sessions and tokens are purged hourly.

Passwords must be 10-128 characters and are rejected if they are on a common-password list (the UK NCSC top 100k, entries of 10+ characters, embedded in the binary), trivially repetitive, or built from the account's own email.

Security-relevant events (sign-up, verification, sign-in success/failure/lockout, token theft, resets, logout) are written to `auth_events` for a year; addresses are stored only as a SHA-256. Emails go through a durable outbox (`email_outbox`): they are retried with backoff if the provider is down, a link stops being retried once it would have expired, and the body (which holds a live link) is blanked as soon as the row is sent or abandoned.

Every response carries an `x-request-id` (a caller-supplied one is kept) that is attached to all log lines for that request. Set `LOG_FORMAT=json` for structured logs.

## Video API

Creators (and admins) upload straight to object storage; the API only coordinates.

| Endpoint | Purpose |
| --- | --- |
| `POST /v1/videos` | Declare filename, size and content type. Returns the video and presigned URLs for each part (16 MiB parts, at most 256) |
| `PUT <part url>` | Done by the browser, directly against storage. Each URL has the part's exact length signed in, so storage refuses any other size |
| `GET /v1/videos/:id/upload` | After an interruption or URL expiry: which parts storage holds, and fresh URLs for only the missing ones |
| `POST /v1/videos/:id/complete` | Verifies every part against storage (never client-supplied ETags), assembles the file and queues transcoding. Safe to retry |
| `POST /v1/videos/:id/abort` | Cancel an unfinished upload |
| `GET /v1/videos/:id` | Status (`uploading`, `processing`, `ready`, `failed`), error code, renditions. Someone else's video looks like a missing one |
| `DELETE /v1/videos/:id` | Stops serving the video at once; an hourly sweep then removes its files (waiting for any still-running transcode job to end first). Cancels an unfinished upload |

Unfinished uploads are cancelled after 24 hours. Original uploads are deleted `SOURCE_RETENTION_DAYS` (default 7) after a video is ready or has failed; only the encrypted renditions remain.

### Transcoding

`zeno-worker` claims `transcode` jobs from Postgres (`FOR UPDATE SKIP LOCKED` under a lease it keeps extending; updates are fenced by attempt number so a worker that lost its lease cannot touch the job). For each video it probes the file, rejects anything unsupported, then writes AES-128 encrypted VOD HLS: 144p, 240p, 360p, 480p and 720p (never above the source; rotation and non-square pixels handled; frame rate capped at 30), plus an audio-only rendition, with keyframes aligned every 6 seconds on every rung. The content key is stored in `video_keys`, never beside the segments, and wrapped with AES-256-GCM under `CONTENT_KEY_SECRET` (bound to the video id), so a copy of the database does not unlock the stored segments; the playlists carry a placeholder key URI that playback replaces per viewer.

Uploads are untrusted input to ffmpeg, so only an allowlist of media containers is accepted (playlists, concat scripts and other formats that reference further resources are refused before any encode); ffmpeg runs with `-protocol_whitelist file`, a scrubbed environment, a hard timeout and kill-on-cancel; output is capped at the duration limit; and metadata, chapters and subtitles are stripped. A file that cannot be processed fails for good with a short code (`unsupported_media`, `no_video_stream`, `too_long`, `resolution_too_high`); infrastructure trouble is retried with backoff.

## Course API

Creators and admins can build courses through a versioned API. Prices are integer paise in INR; slugs are stable after creation; deleting a course is a soft delete so future enrollment and audit history are not orphaned. Updates use optimistic concurrency: send the current `version` in the JSON body or `If-Match`; a stale editor receives `409 version_conflict` instead of silently overwriting newer work.

| Endpoint | Purpose |
| --- | --- |
| `POST /v1/courses` | Create a draft course |
| `GET /v1/courses?limit=&cursor=` | List the signed-in creator's courses using stable keyset pagination |
| `GET /v1/courses/:id` | Read an owned course; another creator's course looks missing |
| `PATCH /v1/courses/:id` | Update metadata and price with optimistic concurrency |
| `POST /v1/courses/:id/publish` | Publish only when every lesson has a ready video |
| `POST /v1/courses/:id/unpublish` | Return a published course to draft |
| `DELETE /v1/courses/:id` | Soft-delete and stop new access |
| `POST /v1/courses/:id/lessons` | Attach an owned processing/ready video; optional insertion position shifts later lessons |
| `GET /v1/courses/:id/lessons` | List dense ordered lessons, video state and computed total duration |
| `PATCH /v1/lessons/:id` | Update lesson metadata, preview flag or attached owned video |
| `DELETE /v1/lessons/:id` | Soft-delete a lesson and close the ordering gap |
| `PUT /v1/courses/:id/lessons/order` | Atomically replace the complete lesson order |
| `POST /v1/courses/:id/enroll` | Idempotently claim a published free course; paid courses point to checkout |
| `POST /v1/courses/:id/grants` | Grant an existing user access or email a pending invitation |
| `DELETE /v1/courses/:id/grants/:user` | Revoke a grant and end sessions that no longer have another access path |
| `GET /v1/courses/:id/students` | Paginated creator view of enrollment history and active state |
| `GET /v1/me/courses` | Paginated courses and entitlement state for the signed-in student |

Lesson positions are dense and all ordering mutations serialize on the course row. Reorder accepts every active lesson ID exactly once; malformed sets return `422` without changing anything, and replaying the current order is a no-op. A processing video can be arranged while encoding, but publishing requires at least one active lesson and every video to be ready. A readiness-breaking lesson mutation—or deleting a video used by an active lesson—automatically returns every affected published course to draft and bumps its version. Lesson deletion retains a tombstone for future progress and analytics history.

Free claims preserve revocation: a student removed by a creator or admin receives `409 access_revoked_by_creator` until an authorised manual grant restores access. Other revoked enrollments also cannot be reactivated through the free endpoint, and expired paid/manual entitlements cannot be converted into free claims. Revocations retain the actor, policy kind and audit history. Pending invitations are accepted by verifying the invited email; invitation emails contain a registration link without a bearer token.

## Public catalog

No account is needed to browse. These endpoints return a whitelisted projection (never a database row), only ever reach published, undeleted courses, and answer `404` identically for "hidden" and "never existed". They are cacheable: `Cache-Control: public, max-age=60, stale-while-revalidate=300` plus a content-derived `ETag` (send `If-None-Match` to get an empty `304`).

| Endpoint | Purpose |
| --- | --- |
| `GET /v1/public/courses` | Browse and search. `q` (up to 100 characters), `language`, `level`, `free=true`, `min_price`, `max_price` (paise), `sort=relevance\|newest\|price_asc\|price_desc`, `page` (1-50), `limit` (1-24, default 12). Answers `{courses, page, has_more}` |
| `GET /v1/public/courses/:slug` | The course page: description, price, creator, lesson titles and durations. A lesson carries its `lesson_id` only when it is a free preview, since that is the only one a visitor can play. No ids, versions, status or storage keys appear |
| `GET /v1/public/creators/:handle` | A creator's profile and their published courses |

Search uses Postgres full-text search with the `simple` configuration (no stemming, so mixed Hindi and English text works), ranked with title matches above short-description above long-description, plus typo tolerance on titles for queries of four or more characters. The query is parsed with `websearch_to_tsquery`, which accepts any text and never errors, so quotes, operators and SQL fragments are inert. Unknown query parameters (campaign tags on shared links) are ignored; malformed known ones are `400`/`422` with the usual error body.

A creator without a profile can still sell: the page then shows their account name and no handle.

### Creator profile and images

| Endpoint | Purpose |
| --- | --- |
| `GET /v1/me/profile` | Your profile, or `null` |
| `PUT /v1/me/profile` | Create or update: `handle`, `display_name`, `bio` (500 characters), up to five `links` (plain `https://` only). A handle is lowercase letters, digits and single hyphens (3-30), not a reserved word, unique, and **permanent** once set because shared links use it (`409 handle_locked`) |
| `PUT /v1/me/profile/avatar`, `DELETE ...` | Set or remove the picture. Needs a profile first |
| `PUT /v1/courses/:id/thumbnail`, `DELETE ...` | Set or remove a course thumbnail. Optional `If-Match: <version>`; a change bumps the course version |

Image uploads are the raw image as the body (`Content-Type: image/jpeg`, `image/png` or `image/webp`, at most 5 MB). Nothing a creator sends is stored or served as sent: the bytes are sniffed (the declared type is not believed), decoded under hard limits (4096 pixels a side, 128 MiB of memory, four decodes at once), rotated by EXIF orientation, centre-cropped, scaled down (never up) and re-encoded as a fresh JPEG, which drops all metadata and anything that was not pixels. SVG and GIF are refused. Thumbnails end up 1280 x 720 at most, avatars 512 x 512. Each upload gets an immutable key containing its content hash and a unique upload id (`images/courses/<id>/<hash>-<upload-id>.jpg`). Replaced and failed uploads are queued for cleanup after 24 hours; the worker checks database references before deleting. With `IMAGE_BASE_URL` set (a CDN in front of the bucket) image URLs are stable while referenced and cacheable; without it they are presigned links good for an hour.

## Playback

| Endpoint | Purpose |
| --- | --- |
| `POST /v1/videos/:id/play` | Start a playback session. Returns the master playlist URL (carrying a signed token), where to resume, the heartbeat interval and the watermark text |
| `GET /v1/play/:session/master.m3u8?t=` | The master playlist, rewritten for this viewer. Authorised by the token alone (players send no cookies) |
| `GET /v1/play/:session/:rendition.m3u8?t=` | A rendition's playlist: segments become presigned storage URLs valid for the session, the key URI points at the key server |
| `POST /v1/play/:session/heartbeat` | Every 30 s with `{"position_seconds": n}`: keeps the session alive and saves the position for resume |
| `DELETE /v1/play/:session` | Stop watching and free the screen |
| `GET /v1/keys/:video_id?t=` | `zeno-keyserver`: the 16-byte AES key, `no-store` |

Who may watch is decided by one database access rule. Creators and admins may preview their videos directly through `POST /v1/videos/:id/play`; students start through `POST /v1/lessons/:id/play` and need either a published free-preview lesson or an active enrollment (`not revoked` and `not expired`) in any course containing that video. Existing enrollments keep working while a creator temporarily unpublishes to edit, and for 30 days after a course is soft-deleted; new claims require a published course. Raw video IDs never act as a student access API.

The playback token (HMAC-SHA256, `PLAYBACK_TOKEN_SECRET`, shared by API and key server) names one session, video and user and expires with the session (twice the video's length plus an hour, between 2 and 12 hours). A token is never enough on its own: every playlist, heartbeat and key request re-checks both the open session and the current entitlement. Revocation ends inaccessible sessions with `access_revoked`; expiration ends them with `expired`; another valid course or free-preview path keeps the shared video playable.

An account may have `MAX_CONCURRENT_STREAMS` (default 2) sessions open. A screen silent for 3 minutes gives up its slot to the next one to press play and is told why when it returns; simultaneous presses are serialised per account, so the limit can't be overshot. The key server only answers browser requests from the web app's origin (a key URL pasted into a player on another site is refused), caps key fetches per session, and applies Redis-backed per-address and per-user limits before releasing a key. It fails closed if that limiter is unavailable. The content key is unwrapped only at that moment. Stored playlists are rewritten strictly: an unknown rendition, a foreign URI, a segment name with a path in it or a playlist without encryption is refused rather than passed on.

Playback and key responses are private, `no-store`, and carry `Referrer-Policy: no-referrer`; request tracing records paths but never query strings, so playback tokens are not written to application logs. For zero-downtime secret rotation, configure the versioned `PLAYBACK_TOKEN_SECRETS` and `CONTENT_KEY_SECRETS` keyrings documented in `.env.example`: new material uses the active version while retained versions continue to verify/open existing sessions and videos.

The watermark (`name · phone or email · session code`) is for the player to draw over the video and keep moving; it is what makes a screen recording traceable to an account and a session.

Other endpoints from the plan answer `501` until they are built. Not built yet: phone OTP sign-in, 2FA, Google sign-in, CAPTCHA, a production mail transport (the API refuses to start in production until one exists).

## Status

Stage 1: foundation, auth, video upload, the transcode pipeline, secure playback, versioned course/lesson authoring, enrollment, grants and student access control, and the public catalog with creator profiles and images are done. Next in the plan: progress tracking and the rate limiter, then checkout.
