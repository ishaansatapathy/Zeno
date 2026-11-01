# Zeno

Secure video and live-class platform for Indian educators. Full plan: [docs/Zeno_Product_and_Technical_Plan.pdf](docs/Zeno_Product_and_Technical_Plan.pdf).

## Layout

```
crates/
  core/        shared types, config, errors
  api/         axum HTTP server: auth, courses, videos, playback manifests, webhooks
  worker/      transcode job runner (FFmpeg)
  live/        stream-key check and live HLS control
  keyserver/   token validation and AES-128 key delivery
web/           Next.js creator dashboard and student app
deploy/        docker-compose, migrations
tests/e2e/     Playwright end-to-end suites
```

## Running locally

```
docker compose -f deploy/docker-compose.yml up -d   # postgres + redis
cargo run -p zeno-api                                 # http://localhost:8080/health
cargo run -p zeno-keyserver
cargo run -p zeno-live
cargo run -p zeno-worker

cd web && npm install && npm run dev                  # http://localhost:3000
```

Apply `deploy/migrations/0001_init.sql` against the `zeno` database before starting the API.

## Status

Scaffold only: route shapes match the planned REST API, handlers are stubs. Build order
follows the Stage 1 roadmap in the plan — auth and DB foundation first, then video core,
commerce and protection, then live classes.
