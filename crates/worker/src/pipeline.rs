//! The transcode job: download the upload, vet it, encode every rendition into AES-128
//! encrypted HLS, publish the result and mark the video ready.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rand::rngs::OsRng;
use rand::RngCore;
use serde_json::json;
use sqlx::PgPool;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use zeno_core::jobs::{self, Job};
use zeno_core::model::VideoStatus;
use zeno_core::storage::{S3Store, StorageError};

use crate::config::WorkerConfig;
use crate::error::JobError;
use crate::ffmpeg::{self, RunError};
use crate::hls::{self, EncodeInput, Segment, Variant};
use crate::ladder::{self, Rung};
use crate::probe::{self, Probe, VideoInfo};

const PROBE_TIMEOUT: Duration = Duration::from_secs(60);
const PROBE_MAX_OUTPUT: usize = 1024 * 1024;
const UPLOAD_CONCURRENCY: usize = 4;
const UPLOAD_ATTEMPTS: u32 = 3;
const MAX_ENCODE_TIMEOUT: Duration = Duration::from_secs(4 * 3600);
const MASTER_NAME: &str = "master.m3u8";

#[derive(Clone)]
pub struct Context {
    pub db: PgPool,
    pub store: S3Store,
    pub config: WorkerConfig,
}

#[derive(sqlx::FromRow)]
struct VideoRow {
    source_key: String,
    status: VideoStatus,
}

fn check(cancel: &CancellationToken) -> Result<(), JobError> {
    if cancel.is_cancelled() {
        Err(JobError::Cancelled)
    } else {
        Ok(())
    }
}

fn video_id_of(job: &Job) -> Result<Uuid, JobError> {
    job.payload
        .get("video_id")
        .and_then(|v| v.as_str())
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or_else(|| JobError::permanent("processing_failed", "job payload has no video_id"))
}

pub async fn process(ctx: &Context, job: &Job, cancel: &CancellationToken) -> Result<(), JobError> {
    let started = Instant::now();
    let video_id = video_id_of(job)?;
    let video: Option<VideoRow> =
        sqlx::query_as("select source_key, status from videos where id = $1")
            .bind(video_id)
            .fetch_optional(&ctx.db)
            .await?;
    let Some(video) = video.filter(|v| v.status == VideoStatus::Processing) else {
        // Deleted, or already handled by an earlier attempt: nothing left to do.
        return finish_without_work(ctx, job).await;
    };

    tokio::fs::create_dir_all(&ctx.config.work_dir).await?;
    let work = tempfile::Builder::new()
        .prefix("job-")
        .tempdir_in(&ctx.config.work_dir)?;
    let source = work.path().join("source");
    ctx.store
        .download_to_file(
            &video.source_key,
            &source,
            ctx.config.limits.max_source_bytes,
        )
        .await?;
    check(cancel)?;

    let probe = probe_file(ctx, &source, cancel).await?;
    let info = probe::validate(&probe, &ctx.config.limits)?;
    let rungs = ladder::choose(info.height);

    // A fresh key per attempt: nothing is published until the whole set is uploaded and the
    // key is committed alongside "ready", and a retry overwrites every object it wrote before.
    let mut key = [0u8; 16];
    OsRng.fill_bytes(&mut key);
    let key_path = work.path().join("key.bin");
    tokio::fs::write(&key_path, key).await?;
    let key_info = work.path().join("keyinfo.txt");
    tokio::fs::write(
        &key_info,
        format!(
            "{}\n{}\n",
            hls::KEY_URI_PLACEHOLDER,
            hls::slashed(&key_path)
        ),
    )
    .await?;

    let encoded = encode_all(
        ctx,
        &probe,
        &info,
        &rungs,
        &source,
        &key_info,
        work.path(),
        cancel,
    )
    .await?;
    check(cancel)?;

    let out = work.path().join("out");
    let master = hls::master_playlist(&encoded.variants);
    tokio::fs::write(out.join(MASTER_NAME), master).await?;
    publish(ctx, video_id, &out, cancel).await?;
    check(cancel)?;

    finalize(ctx, job, video_id, &key, &info, &encoded).await?;
    tracing::info!(
        %video_id,
        renditions = encoded.variants.len(),
        seconds = started.elapsed().as_secs_f64(),
        "video is ready"
    );
    Ok(())
}

async fn finish_without_work(ctx: &Context, job: &Job) -> Result<(), JobError> {
    if jobs::complete(&ctx.db, job).await? {
        Ok(())
    } else {
        Err(JobError::Cancelled)
    }
}

async fn probe_file(
    ctx: &Context,
    source: &Path,
    cancel: &CancellationToken,
) -> Result<Probe, JobError> {
    let args: Vec<String> = [
        "-v",
        "error",
        "-protocol_whitelist",
        "file",
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .chain([format!("file:{}", hls::slashed(source))])
    .collect();
    match ffmpeg::run(
        &ctx.config.ffprobe_bin,
        &args,
        None,
        PROBE_TIMEOUT,
        cancel,
        PROBE_MAX_OUTPUT,
    )
    .await
    {
        Ok(output) => probe::parse(&String::from_utf8_lossy(&output.stdout)),
        Err(RunError::Spawn(err)) => {
            Err(JobError::Transient(format!("cannot start ffprobe: {err}")))
        }
        Err(RunError::Cancelled) => Err(JobError::Cancelled),
        Err(RunError::Timeout) => Err(JobError::permanent(
            "unsupported_media",
            "ffprobe timed out",
        )),
        Err(RunError::Failed { stderr_tail, .. }) => Err(JobError::permanent(
            "unsupported_media",
            format!("ffprobe could not read the file: {stderr_tail}"),
        )),
    }
}

struct Encoded {
    variants: Vec<Variant>,
    duration_secs: f64,
}

fn encode_timeout(ctx: &Context, duration: Option<f64>) -> Duration {
    if let Some(fixed) = ctx.config.encode_timeout {
        return fixed;
    }
    // veryfast encodes comfortably faster than real time; allow generous slack on top.
    let secs = duration.unwrap_or(ctx.config.limits.max_duration_secs) * 4.0 + 300.0;
    Duration::from_secs_f64(secs).min(MAX_ENCODE_TIMEOUT)
}

#[allow(clippy::too_many_arguments)]
async fn encode_all(
    ctx: &Context,
    probe: &Probe,
    info: &VideoInfo,
    rungs: &[Rung],
    source: &Path,
    key_info: &Path,
    work: &Path,
    cancel: &CancellationToken,
) -> Result<Encoded, JobError> {
    let input = EncodeInput {
        source,
        key_info,
        max_duration_secs: ctx.config.limits.max_duration_secs,
        threads: ctx.config.encode_threads,
    };
    let timeout = encode_timeout(ctx, probe.duration_secs);
    let reduce_fps = info.fps.is_some_and(|fps| fps > hls::FPS_CAP);
    let out = work.join("out");
    let mut variants = Vec::new();
    let mut duration_secs = 0.0;

    for rung in rungs {
        check(cancel)?;
        let dir = out.join(&rung.name);
        let args = hls::video_args(&input, rung, probe.has_audio, reduce_fps);
        let segments = encode_one(ctx, &args, &dir, timeout, cancel).await?;
        let measured = measure_segments(&dir, &segments).await?;
        let (peak_bps, average_bps) =
            hls::measure(&measured, hls::configured_peak_bps(rung, probe.has_audio));
        if duration_secs == 0.0 {
            duration_secs = segments.iter().map(|s| s.duration_secs).sum();
        }
        variants.push(Variant {
            name: rung.name.clone(),
            resolution: Some((hls::variant_width(rung, info.display_aspect), rung.height)),
            frame_rate: info.fps.map(|fps| if reduce_fps { 30.0 } else { fps }),
            has_audio: probe.has_audio,
            peak_bps,
            average_bps,
            playlist: format!("{}/{}", rung.name, hls::PLAYLIST_NAME),
        });
    }

    if probe.has_audio {
        check(cancel)?;
        let dir = out.join("audio");
        let args = hls::audio_only_args(&input);
        let segments = encode_one(ctx, &args, &dir, timeout, cancel).await?;
        let measured = measure_segments(&dir, &segments).await?;
        let (peak_bps, average_bps) =
            hls::measure(&measured, hls::audio_only_configured_peak_bps());
        variants.push(Variant {
            name: "audio".to_owned(),
            resolution: None,
            frame_rate: None,
            has_audio: true,
            peak_bps,
            average_bps,
            playlist: format!("audio/{}", hls::PLAYLIST_NAME),
        });
    }

    // The encode is capped at the limit, so reaching it means the source was longer.
    if duration_secs > ctx.config.limits.max_duration_secs + 1.0 {
        return Err(JobError::permanent(
            "too_long",
            format!(
                "{duration_secs:.0}s exceeds the {}s limit",
                ctx.config.limits.max_duration_secs
            ),
        ));
    }
    Ok(Encoded {
        variants,
        duration_secs,
    })
}

async fn encode_one(
    ctx: &Context,
    args: &[String],
    dir: &Path,
    timeout: Duration,
    cancel: &CancellationToken,
) -> Result<Vec<Segment>, JobError> {
    tokio::fs::create_dir_all(dir).await?;
    match ffmpeg::run(&ctx.config.ffmpeg_bin, args, Some(dir), timeout, cancel, 0).await {
        Ok(_) => {}
        Err(RunError::Spawn(err)) => {
            return Err(JobError::Transient(format!("cannot start ffmpeg: {err}")))
        }
        Err(RunError::Cancelled) => return Err(JobError::Cancelled),
        Err(RunError::Timeout) => return Err(JobError::Encode("ffmpeg timed out".into())),
        Err(RunError::Failed { code, stderr_tail }) => {
            return Err(JobError::Encode(format!(
                "ffmpeg exited with {code:?}: {stderr_tail}"
            )))
        }
    }
    let playlist = tokio::fs::read_to_string(dir.join(hls::PLAYLIST_NAME)).await?;
    let segments = hls::parse_media_playlist(&playlist);
    if segments.is_empty() {
        return Err(JobError::Encode("ffmpeg produced no segments".into()));
    }
    Ok(segments)
}

async fn measure_segments(dir: &Path, segments: &[Segment]) -> Result<Vec<(f64, u64)>, JobError> {
    let mut measured = Vec::with_capacity(segments.len());
    for segment in segments {
        if segment.uri.contains(['/', '\\']) || segment.uri.contains("..") {
            return Err(JobError::Encode(format!(
                "unexpected segment name {:?}",
                segment.uri
            )));
        }
        let size = tokio::fs::metadata(dir.join(&segment.uri)).await?.len();
        measured.push((segment.duration_secs, size));
    }
    Ok(measured)
}

/// Every file under `root` as (path relative to root with `/` separators, absolute path).
async fn collect_files(root: &Path) -> Result<Vec<(String, PathBuf)>, JobError> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if entry.file_type().await?.is_dir() {
                stack.push(path);
            } else if let Ok(relative) = path.strip_prefix(root) {
                files.push((hls::slashed(relative), path));
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Segments and variant playlists first, the master playlist last, so nothing points at
/// objects that are not there yet.
async fn publish(
    ctx: &Context,
    video_id: Uuid,
    out: &Path,
    cancel: &CancellationToken,
) -> Result<(), JobError> {
    let (master, rest): (Vec<_>, Vec<_>) = collect_files(out)
        .await?
        .into_iter()
        .partition(|(relative, _)| relative == MASTER_NAME);
    let prefix = format!("videos/{video_id}");

    let permits = Arc::new(Semaphore::new(UPLOAD_CONCURRENCY));
    let mut uploads = JoinSet::new();
    for (relative, path) in rest {
        let store = ctx.store.clone();
        let key = format!("{prefix}/{relative}");
        let permits = permits.clone();
        uploads.spawn(async move {
            let _permit = permits
                .acquire()
                .await
                .map_err(|e| JobError::Transient(e.to_string()))?;
            upload_file(&store, &key, &path).await
        });
    }
    while let Some(result) = uploads.join_next().await {
        check(cancel)?;
        result.map_err(|e| JobError::Transient(format!("upload task: {e}")))??;
    }
    for (relative, path) in master {
        upload_file(&ctx.store, &format!("{prefix}/{relative}"), &path).await?;
    }
    Ok(())
}

async fn upload_file(store: &S3Store, key: &str, path: &Path) -> Result<(), JobError> {
    let bytes = tokio::fs::read(path).await?;
    let content_type = hls::content_type_for(key);
    let mut attempt = 1;
    loop {
        match store.put_bytes(key, bytes.clone(), content_type).await {
            Ok(()) => return Ok(()),
            Err(
                err @ (StorageError::Transport(_)
                | StorageError::Http {
                    status: 500..=599, ..
                }),
            ) if attempt < UPLOAD_ATTEMPTS => {
                tracing::warn!(error = %err, attempt, "upload failed; retrying");
                tokio::time::sleep(Duration::from_millis(500 * u64::from(attempt))).await;
                attempt += 1;
            }
            Err(err) => return Err(err.into()),
        }
    }
}

async fn finalize(
    ctx: &Context,
    job: &Job,
    video_id: Uuid,
    key: &[u8; 16],
    info: &VideoInfo,
    encoded: &Encoded,
) -> Result<(), JobError> {
    let renditions: Vec<_> = encoded
        .variants
        .iter()
        .map(|v| {
            json!({
                "name": v.name,
                "audio_only": v.resolution.is_none(),
                "width": v.resolution.map(|r| r.0),
                "height": v.resolution.map(|r| r.1),
                "bandwidth": v.peak_bps,
                "playlist": v.playlist,
            })
        })
        .collect();

    let mut tx = ctx.db.begin().await?;
    sqlx::query(
        "insert into video_keys (video_id, key) values ($1, $2) \
         on conflict (video_id) do update set key = excluded.key, created_at = now()",
    )
    .bind(video_id)
    .bind(&key[..])
    .execute(&mut *tx)
    .await?;
    let updated = sqlx::query(
        "update videos set status = 'ready', error_code = null, duration_seconds = $2, width = $3, \
         height = $4, renditions = $5, ready_at = now(), updated_at = now() \
         where id = $1 and status = 'processing'",
    )
    .bind(video_id)
    .bind(encoded.duration_secs.round() as i32)
    .bind(i32::try_from(info.width).unwrap_or(i32::MAX))
    .bind(i32::try_from(info.height).unwrap_or(i32::MAX))
    .bind(serde_json::Value::Array(renditions))
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if updated != 1 {
        // The video changed under us (deleted, say). Publishing nothing is the safe outcome.
        tx.rollback().await?;
        return finish_without_work(ctx, job).await;
    }
    if !jobs::complete(&mut *tx, job).await? {
        return Err(JobError::Cancelled);
    }
    tx.commit().await?;
    Ok(())
}
