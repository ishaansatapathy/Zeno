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

Needs Rust (rustup), Docker, and Node 20+.

```
cp .env.example .env              # then set JWT_ACCESS_SECRET (openssl rand -base64 48)
docker compose -f deploy/docker-compose.yml up -d --wait   # Postgres :5433, Redis :6379
cargo run -p zeno-api             # http://localhost:8080/health  (applies migrations in development)

cd web && npm install && npm run dev   # http://localhost:3000
```

In development emails are not sent; the verification and reset links are printed in the API log.

Tests (the integration suites use a real Postgres, each test in its own throwaway database):

```
export DATABASE_URL=postgres://zeno:zeno@127.0.0.1:5433/zeno
export TEST_REDIS_URL=redis://127.0.0.1:6379
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

Other endpoints from the plan answer `501` until they are built. Not built yet: phone OTP sign-in, 2FA, Google sign-in, CAPTCHA, a production mail transport (the API refuses to start in production until one exists).

## Status

Stage 1, foundation and auth done. Next in the plan: video upload and the transcode pipeline.
