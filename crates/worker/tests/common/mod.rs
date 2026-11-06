#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;
use zeno_core::jobs;
use zeno_core::storage::{S3Store, StorageConfig};
use zeno_worker::config::{Limits, WorkerConfig};
use zeno_worker::pipeline::Context;

pub fn ffmpeg_bin() -> String {
    std::env::var("FFMPEG_BIN").unwrap_or_else(|_| "ffmpeg".into())
}

pub fn ffprobe_bin() -> String {
    std::env::var("FFPROBE_BIN").unwrap_or_else(|_| "ffprobe".into())
}

pub fn storage_config() -> StorageConfig {
    let _ = dotenvy::dotenv();
    StorageConfig::from_env()
        .expect("S3_* settings are required for worker tests; see .env.example")
}

pub fn test_config() -> WorkerConfig {
    let work_dir = std::env::temp_dir().join(format!("zeno-worker-test-{}", Uuid::new_v4()));
    WorkerConfig {
        database_url: String::new(),
        storage: storage_config(),
        ffmpeg_bin: ffmpeg_bin(),
        ffprobe_bin: ffprobe_bin(),
        work_dir,
        limits: Limits {
            max_duration_secs: 3600.0,
            max_dimension: 4096,
            max_source_bytes: 512 * 1024 * 1024,
        },
        concurrency: 1,
        encode_threads: 2,
        encode_timeout: None,
    }
}

pub async fn context(pool: PgPool) -> Context {
    context_with(pool, |_| {}).await
}

pub async fn context_with(pool: PgPool, tweak: impl FnOnce(&mut WorkerConfig)) -> Context {
    let mut config = test_config();
    tweak(&mut config);
    let store = S3Store::new(&config.storage).unwrap();
    store
        .ensure_bucket()
        .await
        .expect("storage must be reachable");
    Context {
        db: pool,
        store,
        config,
    }
}

/// A worker whose storage endpoint refuses connections, to exercise outage handling.
pub fn broken_storage_context(pool: PgPool) -> Context {
    let mut config = test_config();
    config.storage.endpoint = "http://127.0.0.1:1".parse().unwrap();
    config.storage.public_endpoint = config.storage.endpoint.clone();
    let store = S3Store::new(&config.storage).unwrap();
    Context {
        db: pool,
        store,
        config,
    }
}

#[derive(Clone)]
pub struct Spec {
    pub width: u32,
    pub height: u32,
    pub secs: u32,
    pub audio: bool,
    pub fps: u32,
}

impl Default for Spec {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
            secs: 3,
            audio: true,
            fps: 25,
        }
    }
}

fn run_ffmpeg(args: &[&str]) {
    let output = Command::new(ffmpeg_bin())
        .args(["-y", "-v", "error", "-nostdin"])
        .args(args)
        .output()
        .expect("ffmpeg must be installed for worker tests");
    assert!(
        output.status.success(),
        "ffmpeg {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A real test-pattern video, made by ffmpeg itself so no binary fixtures live in the repo.
pub fn make_video(dir: &Path, name: &str, spec: &Spec) -> Vec<u8> {
    let path = dir.join(name);
    let video = format!(
        "testsrc=size={}x{}:rate={}:duration={}",
        spec.width, spec.height, spec.fps, spec.secs
    );
    let audio = format!("sine=frequency=440:duration={}", spec.secs);
    let mut args = vec!["-f", "lavfi", "-i", &video];
    if spec.audio {
        args.extend(["-f", "lavfi", "-i", &audio]);
    }
    args.extend([
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-preset",
        "ultrafast",
    ]);
    if spec.audio {
        args.extend(["-c:a", "aac", "-shortest"]);
    }
    let out = path.to_string_lossy().into_owned();
    args.push(&out);
    run_ffmpeg(&args);
    std::fs::read(path).unwrap()
}

/// A real, decodable video in a container the platform does not accept (for example "nut").
pub fn make_in_container(dir: &Path, name: &str, container: &str) -> Vec<u8> {
    let out = dir.join(name).to_string_lossy().into_owned();
    run_ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc=size=320x180:rate=25:duration=2",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-preset",
        "ultrafast",
        "-f",
        container,
        &out,
    ]);
    std::fs::read(dir.join(name)).unwrap()
}

/// Same picture, but marked as rotated by `degrees` (counter-clockwise), the way phones record
/// portrait video.
pub fn make_rotated(dir: &Path, name: &str, spec: &Spec, degrees: i32) -> Vec<u8> {
    make_video(dir, "unrotated.mp4", spec);
    let input = dir.join("unrotated.mp4").to_string_lossy().into_owned();
    let out = dir.join(name).to_string_lossy().into_owned();
    run_ffmpeg(&[
        "-display_rotation:v:0",
        &degrees.to_string(),
        "-i",
        &input,
        "-c",
        "copy",
        &out,
    ]);
    std::fs::read(dir.join(name)).unwrap()
}

pub fn make_audio_only(dir: &Path, name: &str) -> Vec<u8> {
    let out = dir.join(name).to_string_lossy().into_owned();
    run_ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=2",
        "-c:a",
        "aac",
        &out,
    ]);
    std::fs::read(dir.join(name)).unwrap()
}

pub fn scratch_dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

/// Creates an owner, a video row in `processing` with its source uploaded, and a transcode job.
pub async fn seed(ctx: &Context, source: Vec<u8>) -> Uuid {
    let owner: Uuid = sqlx::query_scalar(
        "insert into users (name, email, password_hash, role) values ('T', $1, 'x', 'creator') returning id",
    )
    .bind(format!("{}@x.in", Uuid::new_v4()))
    .fetch_one(&ctx.db)
    .await
    .unwrap();
    let id = Uuid::new_v4();
    let key = format!("uploads/{id}/source");
    ctx.store
        .put_bytes(&key, source.clone(), "application/octet-stream")
        .await
        .unwrap();
    sqlx::query(
        "insert into videos (id, owner_id, title, original_filename, content_type, size_bytes, part_size, source_key, status) \
         values ($1, $2, 't', 'f.mp4', 'video/mp4', $3, 5242880, $4, 'processing')",
    )
    .bind(id)
    .bind(owner)
    .bind(source.len() as i64)
    .bind(key)
    .execute(&ctx.db)
    .await
    .unwrap();
    jobs::enqueue(&ctx.db, "transcode", &serde_json::json!({ "video_id": id }))
        .await
        .unwrap()
        .unwrap();
    id
}

pub struct VideoState {
    pub status: String,
    pub error_code: Option<String>,
    pub duration: Option<i32>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub renditions: Value,
}

pub async fn video_state(pool: &PgPool, id: Uuid) -> VideoState {
    let row: (String, Option<String>, Option<i32>, Option<i32>, Option<i32>, Value) = sqlx::query_as(
        "select status::text, error_code, duration_seconds, width, height, renditions from videos where id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap();
    VideoState {
        status: row.0,
        error_code: row.1,
        duration: row.2,
        width: row.3,
        height: row.4,
        renditions: row.5,
    }
}

pub fn rendition_names(state: &VideoState) -> Vec<String> {
    state
        .renditions
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap().to_owned())
        .collect()
}

pub async fn stored_key(pool: &PgPool, id: Uuid) -> Vec<u8> {
    sqlx::query_scalar("select key from video_keys where video_id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Downloads a published variant (playlist + every segment) next to a copy of `key`, with the
/// placeholder key URI pointed at that local file, the way the playback layer would substitute
/// a real key URL.
pub async fn fetch_variant(
    ctx: &Context,
    id: Uuid,
    variant: &str,
    key: &[u8],
    dir: &Path,
) -> PathBuf {
    let target = dir.join(variant);
    std::fs::create_dir_all(&target).unwrap();
    let playlist = ctx
        .store
        .get_bytes(&format!("videos/{id}/{variant}/index.m3u8"), 1 << 20)
        .await
        .unwrap();
    let text = String::from_utf8(playlist).unwrap();
    for line in text.lines().filter(|l| l.ends_with(".ts")) {
        let bytes = ctx
            .store
            .get_bytes(&format!("videos/{id}/{variant}/{line}"), 64 << 20)
            .await
            .unwrap();
        std::fs::write(target.join(line), bytes).unwrap();
    }
    std::fs::write(target.join("key.bin"), key).unwrap();
    let local = text.replace("zeno-key://video", "key.bin");
    let path = target.join("index.m3u8");
    std::fs::write(&path, local).unwrap();
    path
}

pub fn probe_json(path: &Path) -> Value {
    let output = Command::new(ffprobe_bin())
        .args([
            "-v",
            "error",
            "-allowed_extensions",
            "ALL",
            "-protocol_whitelist",
            "file,crypto",
        ])
        .args(["-print_format", "json", "-show_format", "-show_streams"])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "ffprobe: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// Fully decodes a variant. Succeeds only if the segments decrypt with the key beside them.
pub fn decodes_cleanly(path: &Path) -> bool {
    let output = Command::new(ffmpeg_bin())
        .args([
            "-v",
            "error",
            "-nostdin",
            "-allowed_extensions",
            "ALL",
            "-protocol_whitelist",
            "file,crypto",
            "-i",
        ])
        .arg(path)
        .args(["-f", "null", "-"])
        .output()
        .unwrap();
    output.status.success() && output.stderr.is_empty()
}

pub fn stream<'a>(probe: &'a Value, kind: &str) -> Option<&'a Value> {
    probe["streams"]
        .as_array()?
        .iter()
        .find(|s| s["codec_type"] == kind)
}

pub async fn job_row(pool: &PgPool) -> (i32, bool, bool, Option<String>) {
    sqlx::query_as(
        "select attempts, completed_at is not null, failed_at is not null, last_error from jobs order by created_at desc limit 1",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

pub fn dir_is_empty(path: &Path) -> bool {
    std::fs::read_dir(path)
        .map(|mut d| d.next().is_none())
        .unwrap_or(true)
}
