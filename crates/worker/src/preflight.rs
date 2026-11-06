//! Startup checks: fail at boot with a clear message instead of failing every job later.

use std::path::Path;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::ffmpeg::{self, RunError};
use crate::pipeline::Context;

const STALE_AFTER: Duration = Duration::from_secs(60 * 60);

/// ffmpeg must exist and have the H.264 encoder (a build without libx264 would fail every job).
pub async fn check(ctx: &Context) -> anyhow::Result<()> {
    let cancel = CancellationToken::new();
    let timeout = Duration::from_secs(20);
    let ffprobe = ffmpeg::run(
        &ctx.config.ffprobe_bin,
        &["-version".to_owned()],
        None,
        timeout,
        &cancel,
        4096,
    )
    .await;
    describe(&ctx.config.ffprobe_bin, ffprobe.map(|_| ()))?;

    let encoders = ffmpeg::run(
        &ctx.config.ffmpeg_bin,
        &["-hide_banner".to_owned(), "-encoders".to_owned()],
        None,
        timeout,
        &cancel,
        256 * 1024,
    )
    .await;
    match encoders {
        Ok(output) => {
            let text = String::from_utf8_lossy(&output.stdout);
            for needed in ["libx264", " aac "] {
                anyhow::ensure!(
                    text.contains(needed),
                    "{} has no {} encoder",
                    ctx.config.ffmpeg_bin,
                    needed.trim()
                );
            }
            Ok(())
        }
        Err(err) => describe(&ctx.config.ffmpeg_bin, Err(err)),
    }
}

fn describe(bin: &str, result: Result<(), RunError>) -> anyhow::Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(RunError::Spawn(err)) => anyhow::bail!("cannot run {bin}: {err}"),
        Err(other) => anyhow::bail!("{bin} did not respond to -version: {other:?}"),
    }
}

/// Job directories left behind by a worker that was killed mid-encode.
pub async fn remove_stale_work_dirs(work_dir: &Path) {
    let Ok(mut entries) = tokio::fs::read_dir(work_dir).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let is_job_dir = entry.file_name().to_string_lossy().starts_with("job-");
        let age = entry
            .metadata()
            .await
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|modified| modified.elapsed().ok());
        if is_job_dir && age.is_some_and(|age| age > STALE_AFTER) {
            if let Err(err) = tokio::fs::remove_dir_all(entry.path()).await {
                tracing::warn!(error = ?err, "could not remove a stale work directory");
            }
        }
    }
}
