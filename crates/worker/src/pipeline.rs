//! Turning an upload into something people can watch, in two jobs.
//!
//! `transcode` downloads the upload, vets it, encodes ONE rendition (the first worth watching, see
//! `ladder::first_play_index`) into AES-128 encrypted HLS, publishes it and marks the video ready. From that
//! moment the link plays. `transcode_rest` then adds the other renditions one at a time while people are
//! already watching: each is published, then added to the video's row, then to the master playlist, in that
//! order, so a playlist never names a rendition that is not there yet (and a master is never longer than the
//! list of renditions a viewer's session is checked against). A viewer who opened the video before a rendition
//! arrived keeps what they were given; the next one gets the better ladder.
//!
//! They are separate jobs, in separate lanes of the worker, so that one video's remaining renditions never
//! hold up the first play of the next.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rand::rngs::OsRng;
use rand::RngCore;
use sqlx::PgPool;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use zeno_core::jobs::{self, Job};
use zeno_core::model::VideoStatus;
use zeno_core::segment_keys::DEFAULT_ROTATION_SECS;
use zeno_core::storage::{S3Store, StorageError};

use crate::config::WorkerConfig;
use crate::error::JobError;
use crate::ffmpeg::{self, RunError};
use crate::hls::{self, EncodeInput, Segment, Variant};
use crate::ladder::{self, Rung};
use crate::probe::{self, Probe, VideoInfo};
use crate::seal::{self, Locking, SealFailure};

const PROBE_TIMEOUT: Duration = Duration::from_secs(60);
const PROBE_MAX_OUTPUT: usize = 1024 * 1024;
const UPLOAD_CONCURRENCY: usize = 4;
const UPLOAD_ATTEMPTS: u32 = 3;
const MAX_ENCODE_TIMEOUT: Duration = Duration::from_secs(4 * 3600);
const MASTER_NAME: &str = "master.m3u8";
/// The job that adds the remaining renditions to a video that can already be watched.
pub const REST_KIND: &str = "transcode_rest";

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

#[derive(sqlx::FromRow)]
struct RestRow {
    source_key: String,
    status: VideoStatus,
    renditions: serde_json::Value,
    wrapped_key: Option<Vec<u8>>,
    rotation_secs: Option<i32>,
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

/// The first job: one rendition, published, and the video is ready to play.
pub async fn process(ctx: &Context, job: &Job, cancel: &CancellationToken) -> Result<(), JobError> {
    let started = Instant::now();
    let video_id = video_id_of(job)?;
    let video: Option<VideoRow> = sqlx::query_as(
        "select source_key, status from videos where id = $1 and deleted_at is null",
    )
    .bind(video_id)
    .fetch_optional(&ctx.db)
    .await?;
    let Some(video) = video.filter(|v| v.status == VideoStatus::Processing) else {
        // Deleted, or already handled by an earlier attempt: nothing left to do.
        return finish_without_work(ctx, job).await;
    };

    let work = work_dir(ctx).await?;
    let source = work.path().join("source");
    ctx.store
        .download_to_file(
            &video.source_key,
            &source,
            ctx.config.limits.max_source_bytes,
        )
        .await?;
    let downloaded = started.elapsed();
    check(cancel)?;

    let probe = probe_file(ctx, &source, cancel).await?;
    let info = probe::validate(&probe, &ctx.config.limits)?;
    let rungs = ladder::choose(info.height);
    let first = ladder::first_play_index(&rungs);
    let more_to_come = rungs.len() > 1 || probe.has_audio;
    let probed = started.elapsed();

    // A fresh key per attempt: nothing is published until the rendition is uploaded and the key is
    // committed alongside "ready", and a retry overwrites every object it wrote before.
    let mut key = [0u8; 16];
    OsRng.fill_bytes(&mut key);
    // A new video's segments are locked in periods, each with a key derived from this one (`seal`).
    let rotation_secs = Some(DEFAULT_ROTATION_SECS);
    let locking = Locking {
        video_id,
        content_key: &key,
        rotation_secs,
    };

    let input = EncodeInput {
        source: &source,
        max_duration_secs: ctx.config.limits.max_duration_secs,
        threads: ctx.config.encode_threads,
    };
    let timeout = encode_timeout(ctx, probe.duration_secs);
    let out = work.path().join("out");
    let (variant, duration_secs) = encode_video_rendition(
        ctx,
        &probe,
        &info,
        &input,
        locking,
        &rungs[first],
        &out,
        timeout,
        cancel,
    )
    .await?;
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
    check(cancel)?;
    let encoded = started.elapsed();

    tokio::fs::write(
        out.join(MASTER_NAME),
        hls::master_playlist(std::slice::from_ref(&variant)),
    )
    .await?;
    publish(ctx, video_id, &out, cancel).await?;
    check(cancel)?;
    let published = started.elapsed();

    let rendition = variant.name.clone();
    finalize(
        ctx,
        job,
        video_id,
        &key,
        rotation_secs,
        &info,
        &[variant],
        duration_secs,
        more_to_come,
    )
    .await?;
    tracing::info!(
        %video_id,
        %rendition,
        more_to_come,
        download_ms = downloaded.as_millis() as u64,
        probe_ms = (probed - downloaded).as_millis() as u64,
        encode_ms = (encoded - probed).as_millis() as u64,
        publish_ms = (published - encoded).as_millis() as u64,
        total_ms = started.elapsed().as_millis() as u64,
        "video is ready to play"
    );
    Ok(())
}

/// The second job: the renditions the first one left, added one at a time.
pub async fn process_rest(
    ctx: &Context,
    job: &Job,
    cancel: &CancellationToken,
) -> Result<(), JobError> {
    let started = Instant::now();
    let video_id = video_id_of(job)?;
    let row: Option<RestRow> = sqlx::query_as(
        "select v.source_key, v.status, v.renditions, k.wrapped_key, k.rotation_secs \
         from videos v left join video_keys k on k.video_id = v.id \
         where v.id = $1 and v.deleted_at is null",
    )
    .bind(video_id)
    .fetch_optional(&ctx.db)
    .await?;
    let Some(row) = row.filter(|v| v.status == VideoStatus::Ready) else {
        // Deleted, or sent back for another encode: whoever did that owns the renditions now.
        return finish_without_work(ctx, job).await;
    };
    let wrapped = row
        .wrapped_key
        .ok_or_else(|| JobError::permanent("processing_failed", "the video has no content key"))?;
    let key = ctx.config.vault.open(video_id, &wrapped).map_err(|e| {
        JobError::permanent("processing_failed", format!("cannot open the key: {e}"))
    })?;
    let mut have: Vec<Variant> = row
        .renditions
        .as_array()
        .map(|list| list.iter().filter_map(Variant::from_rendition).collect())
        .unwrap_or_default();

    let work = work_dir(ctx).await?;
    let source = work.path().join("source");
    ctx.store
        .download_to_file(&row.source_key, &source, ctx.config.limits.max_source_bytes)
        .await
        .map_err(|err| match err {
            // The original is deleted a week after the video is ready; the video stays as it is.
            StorageError::NotFound => JobError::permanent(
                "processing_failed",
                "the original upload is gone, so the other renditions cannot be made",
            ),
            other => other.into(),
        })?;
    check(cancel)?;
    let probe = probe_file(ctx, &source, cancel).await?;
    let info = probe::validate(&probe, &ctx.config.limits)?;
    let rungs = ladder::choose(info.height);
    let first = ladder::first_play_index(&rungs);

    let todo: Vec<Rung> = ladder::rest_in_order(&rungs, first)
        .into_iter()
        .filter(|rung| !have.iter().any(|v| v.name == rung.name))
        .collect();
    let audio_todo = probe.has_audio && !have.iter().any(|v| v.name == "audio");
    if todo.is_empty() && !audio_todo {
        return finish_without_work(ctx, job).await;
    }

    // A video made before keys were rotated keeps its one key; the others get the periods it was made with.
    let locking = Locking {
        video_id,
        content_key: &key,
        rotation_secs: row.rotation_secs,
    };
    let input = EncodeInput {
        source: &source,
        max_duration_secs: ctx.config.limits.max_duration_secs,
        threads: ctx.config.encode_threads,
    };
    let timeout = encode_timeout(ctx, probe.duration_secs);

    // Each video rung, then the audio-only one: `None` stands for the audio.
    let steps = todo.iter().map(Some).chain(audio_todo.then_some(None));
    for (n, rung) in steps.enumerate() {
        check(cancel)?;
        let out = work.path().join(format!("out-{n}"));
        let variant = match rung {
            Some(rung) => {
                encode_video_rendition(
                    ctx, &probe, &info, &input, locking, rung, &out, timeout, cancel,
                )
                .await?
                .0
            }
            None => encode_audio_rendition(ctx, &input, locking, &out, timeout, cancel).await?,
        };
        publish(ctx, video_id, &out, cancel).await?;
        check(cancel)?;
        let name = variant.name.clone();
        have.push(variant);
        // The row first, then the master: a master never names a rendition the row does not.
        if !record_renditions(ctx, video_id, &wrapped, &have).await? {
            // Deleted, or encoded again with a new key while this was going on.
            return finish_without_work(ctx, job).await;
        }
        publish_master(ctx, video_id, &have).await?;
        tracing::info!(
            %video_id,
            rendition = %name,
            renditions = have.len(),
            seconds = started.elapsed().as_secs_f64(),
            "rendition added"
        );
    }
    finish_without_work(ctx, job).await
}

async fn work_dir(ctx: &Context) -> Result<tempfile::TempDir, JobError> {
    tokio::fs::create_dir_all(&ctx.config.work_dir).await?;
    Ok(tempfile::Builder::new()
        .prefix("job-")
        .tempdir_in(&ctx.config.work_dir)?)
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

fn encode_timeout(ctx: &Context, duration: Option<f64>) -> Duration {
    if let Some(fixed) = ctx.config.encode_timeout {
        return fixed;
    }
    // veryfast encodes comfortably faster than real time; allow generous slack on top.
    let secs = duration.unwrap_or(ctx.config.limits.max_duration_secs) * 4.0 + 300.0;
    Duration::from_secs_f64(secs).min(MAX_ENCODE_TIMEOUT)
}

/// Encodes one picture-and-sound rendition into `out/<name>/`, and describes it. Also returns how long the
/// segments add up to, which is the video's duration.
#[allow(clippy::too_many_arguments)]
async fn encode_video_rendition(
    ctx: &Context,
    probe: &Probe,
    info: &VideoInfo,
    input: &EncodeInput<'_>,
    locking: Locking<'_>,
    rung: &Rung,
    out: &Path,
    timeout: Duration,
    cancel: &CancellationToken,
) -> Result<(Variant, f64), JobError> {
    check(cancel)?;
    let reduce_fps = info.fps.is_some_and(|fps| fps > hls::FPS_CAP);
    let dir = out.join(&rung.name);
    let args = hls::video_args(input, rung, probe.has_audio, reduce_fps);
    let segments = encode_one(ctx, &args, &dir, locking, timeout, cancel).await?;
    let measured = measure_segments(&dir, &segments).await?;
    let (peak_bps, average_bps) =
        hls::measure(&measured, hls::configured_peak_bps(rung, probe.has_audio));
    let duration_secs = segments.iter().map(|s| s.duration_secs).sum();
    let variant = Variant {
        name: rung.name.clone(),
        resolution: Some((hls::variant_width(rung, info.display_aspect), rung.height)),
        frame_rate: info.fps.map(|fps| if reduce_fps { 30.0 } else { fps }),
        has_audio: probe.has_audio,
        peak_bps,
        average_bps,
        playlist: format!("{}/{}", rung.name, hls::PLAYLIST_NAME),
    };
    Ok((variant, duration_secs))
}

/// The audio-only rendition, into `out/audio/`.
async fn encode_audio_rendition(
    ctx: &Context,
    input: &EncodeInput<'_>,
    locking: Locking<'_>,
    out: &Path,
    timeout: Duration,
    cancel: &CancellationToken,
) -> Result<Variant, JobError> {
    check(cancel)?;
    let dir = out.join("audio");
    let args = hls::audio_only_args(input);
    let segments = encode_one(ctx, &args, &dir, locking, timeout, cancel).await?;
    let measured = measure_segments(&dir, &segments).await?;
    let (peak_bps, average_bps) = hls::measure(&measured, hls::audio_only_configured_peak_bps());
    Ok(Variant {
        name: "audio".to_owned(),
        resolution: None,
        frame_rate: None,
        has_audio: true,
        peak_bps,
        average_bps,
        playlist: format!("audio/{}", hls::PLAYLIST_NAME),
    })
}

async fn encode_one(
    ctx: &Context,
    args: &[String],
    dir: &Path,
    locking: Locking<'_>,
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
    // ffmpeg wrote it in the clear; what is published is locked, a key to each period.
    seal::seal_rendition(dir, hls::PLAYLIST_NAME, locking)
        .await
        .map_err(|failure| match failure {
            SealFailure::Io(err) => JobError::from(err),
            SealFailure::Playlist(err) => JobError::Encode(err.to_string()),
        })?;
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

/// Segments and variant playlists first, the master playlist (when there is one in `out`) last, so
/// nothing points at objects that are not there yet.
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

/// Replaces the master playlist with one that lists every rendition in `variants`.
async fn publish_master(
    ctx: &Context,
    video_id: Uuid,
    variants: &[Variant],
) -> Result<(), JobError> {
    let key = format!("videos/{video_id}/{MASTER_NAME}");
    let bytes = hls::master_playlist(variants).into_bytes();
    put_with_retries(&ctx.store, &key, bytes, hls::content_type_for(&key)).await
}

async fn upload_file(store: &S3Store, key: &str, path: &Path) -> Result<(), JobError> {
    let bytes = tokio::fs::read(path).await?;
    put_with_retries(store, key, bytes, hls::content_type_for(key)).await
}

async fn put_with_retries(
    store: &S3Store,
    key: &str,
    bytes: Vec<u8>,
    content_type: &'static str,
) -> Result<(), JobError> {
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

/// The renditions as the video's row describes them, in the order the master lists them.
fn renditions_json(variants: &[Variant]) -> serde_json::Value {
    serde_json::Value::Array(
        hls::ordered(variants)
            .into_iter()
            .map(Variant::to_rendition)
            .collect(),
    )
}

/// Writes the list of renditions into the video's row, but only while the video is still the one this job
/// was started for: not deleted, and still under the content key it read (a video encoded again has a new
/// one, and renditions made under the old key would not play). `false` means it is not.
async fn record_renditions(
    ctx: &Context,
    video_id: Uuid,
    wrapped_key: &[u8],
    variants: &[Variant],
) -> Result<bool, JobError> {
    let updated = sqlx::query(
        "update videos set renditions = $2, updated_at = now() \
         where id = $1 and status = 'ready' and deleted_at is null \
           and exists (select 1 from video_keys where video_id = $1 and wrapped_key = $3)",
    )
    .bind(video_id)
    .bind(renditions_json(variants))
    .bind(wrapped_key)
    .execute(&ctx.db)
    .await?
    .rows_affected();
    Ok(updated == 1)
}

/// Seals the key, marks the video ready with the rendition(s) published so far, finishes this job and, if
/// there is more to make, queues the job that makes it: all in one transaction, so a video is never ready
/// without its key, and never ready with nobody left to finish it.
#[allow(clippy::too_many_arguments)]
async fn finalize(
    ctx: &Context,
    job: &Job,
    video_id: Uuid,
    key: &[u8; 16],
    rotation_secs: Option<i32>,
    info: &VideoInfo,
    variants: &[Variant],
    duration_secs: f64,
    more_to_come: bool,
) -> Result<(), JobError> {
    let wrapped = ctx
        .config
        .vault
        .seal(video_id, key)
        .map_err(|e| JobError::Transient(e.to_string()))?;
    let mut tx = ctx.db.begin().await?;
    sqlx::query(
        "insert into video_keys (video_id, wrapped_key, rotation_secs) values ($1, $2, $3) \
         on conflict (video_id) do update set wrapped_key = excluded.wrapped_key, \
             rotation_secs = excluded.rotation_secs, created_at = now()",
    )
    .bind(video_id)
    .bind(&wrapped[..])
    .bind(rotation_secs)
    .execute(&mut *tx)
    .await?;
    let updated = sqlx::query(
        "update videos set status = 'ready', error_code = null, duration_seconds = $2, width = $3, \
         height = $4, renditions = $5, ready_at = now(), updated_at = now() \
         where id = $1 and status = 'processing' and deleted_at is null",
    )
    .bind(video_id)
    .bind(duration_secs.round() as i32)
    .bind(i32::try_from(info.width).unwrap_or(i32::MAX))
    .bind(i32::try_from(info.height).unwrap_or(i32::MAX))
    .bind(renditions_json(variants))
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
    if more_to_come {
        // After this job is complete, so the one-live-job-per-video rule lets the next one in.
        jobs::enqueue(
            &mut *tx,
            REST_KIND,
            &serde_json::json!({ "video_id": video_id }),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}
