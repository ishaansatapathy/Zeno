//! The whole path, with nothing faked: a real video file is uploaded through the API straight to
//! storage, the real worker encodes it with ffmpeg, a viewer presses play, and ffmpeg itself plays
//! the result over HTTP from a running API and key server, fetching playlists with the viewer's
//! token, segments from presigned storage URLs and the AES key from the key server.

mod common;

use std::net::SocketAddr;
use std::process::Command;
use std::sync::Arc;

use axum::http::StatusCode;
use base64::Engine;
use common::*;
use serde_json::json;
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use zeno_api::mail::MemoryMailer;
use zeno_core::keyvault::KeyVault;
use zeno_core::playback::PlaybackSigner;
use zeno_core::storage::S3Store;
use zeno_keyserver::{KeyLimiter, KeyServer};
use zeno_worker::config::{Limits, WorkerConfig};

fn ffmpeg() -> String {
    std::env::var("FFMPEG_BIN").unwrap_or_else(|_| "ffmpeg".into())
}

fn vault() -> KeyVault {
    KeyVault::from_secret(
        &base64::engine::general_purpose::STANDARD.encode(*b"e2e-content-key-secret-32-bytes!"),
    )
    .unwrap()
}

async fn serve(router: axum::Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    addr
}

fn make_video(dir: &std::path::Path) -> Vec<u8> {
    let out = dir.join("lecture.mp4");
    let status = Command::new(ffmpeg())
        .args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=320x240:rate=25:duration=4",
        ])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=4"])
        .args([
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
            "-y",
        ])
        .arg(&out)
        .status()
        .unwrap();
    assert!(status.success());
    std::fs::read(out).unwrap()
}

/// ffmpeg playing `url` to nowhere; returns (success, stderr). Runs off the test's runtime
/// thread: `sqlx::test` is single-threaded, and the API and key server ffmpeg talks to live on
/// that same thread. `-rw_timeout` turns any stall into a failure instead of a hang.
async fn play_with_ffmpeg(url: &str) -> (bool, String) {
    let url = url.to_owned();
    tokio::task::spawn_blocking(move || {
        let output = Command::new(ffmpeg())
            .args([
                "-nostdin",
                "-v",
                "error",
                "-rw_timeout",
                "15000000",
                "-i",
                &url,
            ])
            .args(["-map", "0:v:0", "-map", "0:a:0", "-f", "null", "-"])
            .output()
            .unwrap();
        (
            output.status.success(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    })
    .await
    .unwrap()
}

#[sqlx::test(migrator = "zeno_core::db::MIGRATOR")]
async fn an_uploaded_lecture_plays_end_to_end_and_only_with_a_live_session(pool: PgPool) {
    // Key server, on a real port.
    let signer = PlaybackSigner::new(PLAYBACK_SECRET).unwrap();
    let keys = serve(zeno_keyserver::router(KeyServer {
        db: pool.clone(),
        vault: vault(),
        tokens: signer.clone(),
        client_origin: ORIGIN.parse().unwrap(),
        limiter: KeyLimiter::memory(10_000, 10_000),
        trusted_proxy_hops: 0,
    }))
    .await;

    // API, on a real port, pointing key URIs at that key server.
    let mailer = MemoryMailer::default();
    let state = make_state_with(pool.clone(), false, 0, Arc::new(mailer.clone()), |config| {
        config.key_server_url = format!("http://{keys}");
    });
    let app = App {
        router: zeno_api::build_app(state.clone()),
        mailer,
        pool: pool.clone(),
        state,
    };
    let api = serve(app.router.clone()).await;

    // Upload a real lecture through the API.
    let scratch = tempfile::tempdir().unwrap();
    let source = make_video(scratch.path());
    let creator = signup_creator(&app, "asha@x.in").await;
    let created = app
        .post("/v1/videos")
        .jar(&creator)
        .json(json!({"filename": "Lecture.mp4", "size_bytes": source.len(), "content_type": "video/mp4"}))
        .send()
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:?}", created.body);
    let video = Uuid::parse_str(created.body["video"]["id"].as_str().unwrap()).unwrap();
    assert!(browser_put(
        created.body["upload"]["parts"][0]["url"].as_str().unwrap(),
        source
    )
    .await
    .is_success());
    let done = app
        .post(&format!("/v1/videos/{video}/complete"))
        .jar(&creator)
        .send()
        .await;
    assert_eq!(done.status, StatusCode::OK, "{:?}", done.body);

    // The real worker encodes it.
    let worker = zeno_worker::pipeline::Context {
        db: pool.clone(),
        store: S3Store::new(&storage_config()).unwrap(),
        config: WorkerConfig {
            database_url: String::new(),
            storage: storage_config(),
            vault: vault(),
            ffmpeg_bin: ffmpeg(),
            ffprobe_bin: std::env::var("FFPROBE_BIN").unwrap_or_else(|_| "ffprobe".into()),
            work_dir: scratch.path().join("work"),
            limits: Limits {
                max_duration_secs: 600.0,
                max_dimension: 4096,
                max_source_bytes: 64 << 20,
            },
            concurrency: 1,
            encode_threads: 2,
            encode_timeout: None,
        },
    };
    assert!(
        zeno_worker::runner::run_once(&worker, &CancellationToken::new())
            .await
            .unwrap()
    );
    let shown = app
        .get(&format!("/v1/videos/{video}"))
        .jar(&creator)
        .send()
        .await;
    assert_eq!(shown.body["video"]["status"], "ready", "{:?}", shown.body);

    // Build and publish a free course, then enroll a real student. The player path below is the
    // student lesson route—not the creator-only raw video preview route.
    let course = app
        .post("/v1/courses")
        .jar(&creator)
        .json(json!({
            "title": "End-to-end course",
            "description": "A real encoded lecture",
            "price_paise": 0
        }))
        .send()
        .await;
    assert_eq!(course.status, StatusCode::CREATED, "{:?}", course.body);
    let course_id = Uuid::parse_str(course.body["course"]["id"].as_str().unwrap()).unwrap();
    let lesson = app
        .post(&format!("/v1/courses/{course_id}/lessons"))
        .jar(&creator)
        .json(json!({"title": "Lecture", "video_id": video}))
        .send()
        .await;
    assert_eq!(lesson.status, StatusCode::CREATED, "{:?}", lesson.body);
    let lesson_id = Uuid::parse_str(lesson.body["lesson"]["id"].as_str().unwrap()).unwrap();
    let published = app
        .post(&format!("/v1/courses/{course_id}/publish"))
        .jar(&creator)
        .json(json!({"version": 2}))
        .send()
        .await;
    assert_eq!(published.status, StatusCode::OK, "{:?}", published.body);
    let student = signup_verified(&app, "student@x.in").await;
    let enrolled = app
        .post(&format!("/v1/courses/{course_id}/enroll"))
        .jar(&student)
        .send()
        .await;
    assert_eq!(enrolled.status, StatusCode::CREATED, "{:?}", enrolled.body);

    // Press play as the student, then let ffmpeg be the player.
    let res = app
        .post(&format!("/v1/lessons/{lesson_id}/play"))
        .jar(&student)
        .json(json!({}))
        .send()
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
    let session = res.body["playback"]["session_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let master = format!(
        "http://{api}{}",
        res.body["playback"]["playlist_url"].as_str().unwrap()
    );
    let (ok, stderr) = play_with_ffmpeg(&master).await;
    assert!(ok && stderr.trim().is_empty(), "playback failed: {stderr}");

    let fetches: i32 =
        sqlx::query_scalar("select key_fetches from playback_sessions where id = $1::uuid")
            .bind(&session)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(fetches >= 1, "the key came from the key server");

    // A token that is not this session's own gets nowhere.
    let token = master.split("?t=").nth(1).unwrap();
    let wrong = master.replace(token, &format!("{}A", &token[..token.len() - 1]));
    assert!(!play_with_ffmpeg(&wrong).await.0);

    // After the viewer stops, the same URL plays no more.
    let stopped = app
        .req(axum::http::Method::DELETE, &format!("/v1/play/{session}"))
        .jar(&student)
        .send()
        .await;
    assert_eq!(stopped.status, StatusCode::NO_CONTENT);
    assert!(
        !play_with_ffmpeg(&master).await.0,
        "a stopped session must not play"
    );
}
