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

Other endpoints from the plan answer `501` until they are built. Not built yet: phone OTP sign-in, 2FA, Google sign-in, CAPTCHA, a production mail transport (the API refuses to start in production until one exists).

## Status

Stage 1: foundation, auth, video upload and the transcode pipeline are done. Next in the plan: playback (signed playlists, key delivery, watermark) and courses.
