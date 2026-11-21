# Zeno

**One place to record, stream, edit, host and share video.** A desktop studio for Windows and
macOS, a browser extension for instant screen recordings, live meetings, and secure hosting with
payments underneath. Built for teachers and creators everywhere. (Zeno is a working name.)

| Surface | What it does | State |
| --- | --- | --- |
| **Hosting** | Upload once; encoded to five qualities (144p to 720p), encrypted segment by segment, played behind short-lived signed tokens. Courses, enrollment, checkout, refunds and a double-entry earnings ledger. A library of recordings on top (named, retry-safe to create, deleted together with their video), and share links: anyone with a link watches with no account, optionally behind a password or an expiry, and revoking one ends every viewing at once. | Built |
| **Web app** | Public site; account screens (sign-up, sign-in, email confirmation, password reset) and the library (list, rename, delete, copy link). Share pages and the player next. | Site, account screens and library built |
| **Extension** | Record screen, camera and mic in Chrome, Edge or Brave; a share link exists before you stop recording. | Planned |
| **Studio** (desktop) | Windows first, then macOS. Record and stream; go live to only the platforms you tick for that stream (YouTube, Twitch, Instagram, X, any RTMP key), wide and tall at once, one merged chat. Pick any camera, including a phone. | Planned |
| **Editor** | Trim, cut, captions, and vertical clips for Shorts and Reels, from any recording or stream. | Planned |
| **Meetings** | Async: record, send a link, get comments. Live: video calls that record straight into the library. | Planned |

The backend reference (every endpoint, limit and guarantee) is in [docs/backend.md](docs/backend.md). How the desktop studio and the editor are built so the user's machine does the work, what that costs us and what it does not, and how the parts scale, is in [docs/studio-design.md](docs/studio-design.md).

---

## The idea the architecture is built on

> **Our servers decide; they never carry the bytes.**

Video is heavy and requests are cheap. So every heavy path is designed as **one small API call that
authorizes the work, after which the bytes go directly between the user and the place that is built
to carry them**: object storage, a CDN, a media server, or the streaming platform itself. The API
signs, counts and records; it never sits in the middle of a video stream. That is what lets a
handful of stateless servers serve very large audiences, and it is how the parts already built work:

| Path | The one call | Where the bytes go | Built |
| --- | --- | --- | --- |
| Upload | `POST /v1/videos` → presigned part URLs | Browser → object storage, in parallel parts, resumable | Yes |
| Encode | Completion enqueues a job (Postgres `SKIP LOCKED`) | Worker ↔ object storage | Yes |
| Watch | `POST /v1/lessons/:id/play` → signed playlist + token | Player ← CDN (segments); a tiny key server checks each key request | Yes |
| Share link | `GET /v1/s/:code` (one indexed lookup, edge-cacheable), then `POST /v1/s/:code/play` | Viewer ← CDN (segments); the key server re-checks the link on every key | Yes |
| Go live to many platforms | `POST /v1/streams` → credentials for each ticked destination | Studio → YouTube, Twitch, Instagram, X directly. **Our egress: zero** (the user's uplink carries every copy; see [docs/studio-design.md](docs/studio-design.md)) | Planned |
| Live on Zeno | Stream key check | Studio → media server → CDN (rolling HLS) | Partly |
| Meeting | Mint a room token | Participants ↔ SFU (LiveKit) | Planned |

The rules that make this hold at scale:

1. **Stateless API.** No session lives in a server's memory, so any number of copies sit behind a
   load balancer. State is in Postgres (truth) and Redis (fast counters, revocation, limits).
2. **Signed, short-lived URLs instead of proxies.** Storage, CDN and key access are granted by a
   signature that expires; nothing has to stream through the API to be protected.
3. **Cache what is public at the edge.** Share pages and catalog reads are cacheable; the database
   sees one request per cache miss, not one per viewer.
4. **Heavy work goes on a queue.** Encoding, clipping, captions and cleanup are jobs with leases,
   retries and fencing, so a crash repeats work instead of losing it.
5. **Everything is idempotent.** Retries are normal: payments are applied exactly once however many
   times the provider reports them; uploads and jobs can be repeated safely.
6. **Money never leaves the ledger.** Double-entry, append-only, balanced by a database trigger,
   checked nightly.
7. **Prove it by breaking it.** Chaos tests kill the database mid-payment; mutation tests check
   that the tests notice real bugs.

## Architecture

```mermaid
flowchart LR
  subgraph Clients
    S["Studio<br/>(Electron, Windows + macOS)"]
    E["Extension<br/>(Chrome MV3)"]
    W["Web app + share pages<br/>(Next.js)"]
  end

  subgraph Zeno
    API["API (Rust, axum)<br/>stateless, horizontally scaled"]
    WK["Workers<br/>encode · clips · captions · cleanup"]
    KS["Key server"]
    LIVE["Live control<br/>+ media server"]
    PG[("Postgres<br/>source of truth")]
    RD[("Redis<br/>limits, counters")]
  end

  OS[("Object storage")]
  CDN(["CDN"])
  SFU(["SFU (LiveKit)"])
  PLAT(["YouTube · Twitch · Instagram · X · any RTMP"])

  S & E & W -- "small signed calls" --> API
  API --- PG & RD
  API -- jobs --> WK
  WK <--> OS
  S & E & W -- "upload parts (presigned)" --> OS
  OS --> CDN
  W -- "segments" --> CDN
  W -- "keys" --> KS
  S -- "RTMP, encoded once on the user's machine" --> PLAT
  S -- "RTMP" --> LIVE --> CDN
  S & W <-- "WebRTC" --> SFU
```

### What each part is

| Part | Language | Notes |
| --- | --- | --- |
| `crates/api` | Rust (axum, sqlx) | Auth, videos, courses, playback, checkout, webhooks, refunds, ledger, payouts. Next: recordings, share links, streams, destinations, meetings. |
| `crates/worker` | Rust + ffmpeg | Transcode to AES-128 HLS (144p to 720p plus audio only). Next: clip and vertical reframe, captions, thumbnails. |
| `crates/keyserver` | Rust | Releases content keys only to an open, entitled session; per-user and per-address limits; fails closed. |
| `crates/live` | Rust + MediaMTX | Stream-key check against the scheduled class; rolling HLS. |
| `crates/core` | Rust | Config, database, job queue, key vault, payment provider trait. |
| `web/` | Next.js 16, React 19 | Public site, account screens and library; share pages next. |
| `apps/studio` | Electron + React, native helpers | Planned. Capture, compose, record, stream; ffmpeg for one encode and many outputs. |
| `apps/extension` | Chrome MV3 | Planned. Records in an offscreen document and uploads parts while recording, so the link is ready at stop. |

### Studio, in a little more detail

- **Go live where you choose.** Connect accounts once (OAuth where the platform allows it: YouTube,
  Twitch; a pasted RTMP URL and key otherwise: Instagram, X, anything else). Each stream, tick the
  destinations; only those go live.
- **Encode once, send many.** The Studio encodes on the user's machine (hardware encoders where
  present) and sends the same stream to every ticked destination. Our servers only hand out
  credentials, so streaming to five places costs us the same as streaming to one.
- **Wide and tall.** One take, a 16:9 output and a 9:16 output.
- **One chat.** Comments from every platform that offers an API for it (YouTube and Twitch fully;
  others as far as their APIs allow) in one feed.
- **Any camera.** Choose the camera at recording time, including a phone over the local network
  (scan a code, no app), for laptops whose built-in webcam is poor.
- **Everything lands in the library.** Streams and recordings upload in the background and become
  hosted, protected, shareable videos.

## Roadmap

About five months, one month at a time. Each month ends with something that runs.

| Month | Milestone |
| --- | --- |
| 1 | Recordings and share links in the API; app pages (sign-in, library, share player); the extension records and returns a link. |
| 2 | Studio on Windows: capture, layouts, local recording, camera choice (including phone), upload to the library. |
| 3 | Live: destinations, multistream from the Studio, wide and tall, merged chat, recordings saved to the library. |
| 4 | Editor: trim, cut, captions, vertical clips. Studio as a virtual camera on Windows. |
| 5 | Live meetings (LiveKit), macOS build and signing, installers and auto-update, paid plans on the existing billing. |

Long-lead paperwork starts in month 1: platform app reviews for YouTube and Twitch sign-in, a
Windows code-signing certificate, and an Apple developer account.

## Layout

```
crates/
  core/        config, Postgres pool + migrator, job queue, key vault, payment trait
  api/         axum HTTP server
  worker/      transcode and media jobs (FFmpeg)
  live/        stream-key check and live HLS control
  keyserver/   token validation and AES-128 key delivery
deploy/        docker-compose, SQL migrations
web/           Next.js site and app (web/e2e: Playwright end-to-end suites)
docs/          backend and web references, the OpenAPI document, runbooks
```

## Running locally

Needs Rust (rustup), Docker, Node 22.12+, and ffmpeg (with libx264) on PATH for the worker.

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
cd web && npm run typecheck && npm test && npm run api:check && npm run build
cd web && npm run e2e                   # real browser against the real API (see docs/web.md)
```

## Status

Built and tested: accounts and auth, resumable uploads, the transcode pipeline, secure playback with
a key server and a screen limit, courses and lessons, enrollment and access control, the public
catalog, progress, rate limits, checkout and webhooks applied exactly once, reconciliation, refunds
capped at what was paid, coupons, invoices (off until tax advice), the earnings ledger and payouts,
and the public site. CI runs the Rust suite, the web build and tests, and a dependency audit on
every push.

Also built: the recordings library (idempotent creation, keyset-paginated, measured at 0.15 ms a page on 200,000 rows), share links (22-character unguessable codes, passwords checked behind a per-link guess limit, revocation that refuses new viewings and keys at once and stops a playing viewer within 30 seconds; about 780 visitor starts a second on a laptop), view counts (counted in Redis, written to Postgres in batches: 10,000 views became 5 rows written, with exact totals), and, in the web app, the account screens and the library (list, rename with conflict handling, delete, copy link), typed from the API's OpenAPI document and covered by browser tests.

Next: the share page and player, then the extension.
