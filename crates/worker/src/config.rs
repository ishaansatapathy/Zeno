use std::path::PathBuf;
use std::time::Duration;

use zeno_core::config::{parse_or, required, ConfigError};
use zeno_core::storage::StorageConfig;

#[derive(Clone)]
pub struct Limits {
    /// Longest video accepted, in seconds.
    pub max_duration_secs: f64,
    /// Largest width or height accepted, in pixels.
    pub max_dimension: u32,
    /// Largest source file downloaded, in bytes.
    pub max_source_bytes: u64,
}

#[derive(Clone)]
pub struct WorkerConfig {
    pub database_url: String,
    pub storage: StorageConfig,
    pub ffmpeg_bin: String,
    pub ffprobe_bin: String,
    /// Scratch space for downloads and encodes; every job gets its own directory here.
    pub work_dir: PathBuf,
    pub limits: Limits,
    /// Transcode jobs run at the same time in this process. Encoding is CPU-bound, so the
    /// default is one.
    pub concurrency: usize,
    /// Threads given to each ffmpeg process (0 lets ffmpeg decide).
    pub encode_threads: u32,
    /// Overrides the computed per-ffmpeg-process timeout. Left unset in production.
    pub encode_timeout: Option<Duration>,
}

impl WorkerConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let work_dir = match std::env::var("WORK_DIR") {
            Ok(dir) if !dir.trim().is_empty() => PathBuf::from(dir),
            _ => std::env::temp_dir().join("zeno-worker"),
        };
        let max_duration_secs: f64 = parse_or("MAX_DURATION_SECS", 6.0 * 3600.0)?;
        if !(max_duration_secs.is_finite() && max_duration_secs > 0.0) {
            return Err(ConfigError::Invalid {
                name: "MAX_DURATION_SECS",
                reason: "must be a positive number".into(),
            });
        }
        let concurrency: usize = parse_or("WORKER_CONCURRENCY", 1)?;
        if concurrency == 0 {
            return Err(ConfigError::Invalid {
                name: "WORKER_CONCURRENCY",
                reason: "must be at least 1".into(),
            });
        }
        Ok(Self {
            database_url: required("DATABASE_URL")?,
            storage: StorageConfig::from_env()?,
            ffmpeg_bin: parse_or("FFMPEG_BIN", "ffmpeg".to_owned())?,
            ffprobe_bin: parse_or("FFPROBE_BIN", "ffprobe".to_owned())?,
            work_dir,
            limits: Limits {
                max_duration_secs,
                max_dimension: parse_or("MAX_DIMENSION", 4096)?,
                max_source_bytes: parse_or("MAX_UPLOAD_BYTES", 10u64 * 1024 * 1024 * 1024)?,
            },
            concurrency,
            encode_threads: parse_or("ENCODE_THREADS", 0)?,
            encode_timeout: parse_or("ENCODE_TIMEOUT_SECS", 0u64)
                .map(|secs| (secs > 0).then(|| Duration::from_secs(secs)))?,
        })
    }
}
