//! Running ffmpeg and ffprobe as child processes, with a hard timeout, cancellation, bounded
//! output and a scrubbed environment. They process untrusted files, so they get nothing from
//! this process's environment and are killed the moment they overrun or the job is cancelled.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

const STDERR_TAIL_BYTES: usize = 4096;

#[derive(Debug)]
pub enum RunError {
    /// The binary could not be started (missing, not executable).
    Spawn(std::io::Error),
    Timeout,
    Cancelled,
    Failed {
        code: Option<i32>,
        stderr_tail: String,
    },
}

#[derive(Debug)]
pub struct Output {
    pub stdout: Vec<u8>,
}

pub async fn run(
    bin: &str,
    args: &[String],
    cwd: Option<&Path>,
    timeout: Duration,
    cancel: &CancellationToken,
    max_stdout: usize,
) -> Result<Output, RunError> {
    let mut command = Command::new(bin);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env_clear();
    for name in ["PATH", "SystemRoot", "TEMP", "TMP", "TMPDIR"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }

    let mut child = command.spawn().map_err(RunError::Spawn)?;
    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");
    let stdout_task = tokio::spawn(read_capped(stdout, max_stdout));
    let stderr_task = tokio::spawn(read_tail(stderr, STDERR_TAIL_BYTES));

    let outcome = tokio::select! {
        status = child.wait() => Ok(status),
        () = tokio::time::sleep(timeout) => Err(RunError::Timeout),
        () = cancel.cancelled() => Err(RunError::Cancelled),
    };
    let status = match outcome {
        Ok(status) => status.map_err(RunError::Spawn)?,
        Err(interrupted) => {
            // Reaping matters: a killed child that is never waited on lingers as a zombie.
            let _ = child.kill().await;
            return Err(interrupted);
        }
    };

    let stdout = stdout_task.await.unwrap_or_default();
    let stderr_tail = stderr_task.await.unwrap_or_default();
    if status.success() {
        Ok(Output { stdout })
    } else {
        Err(RunError::Failed {
            code: status.code(),
            stderr_tail: String::from_utf8_lossy(&stderr_tail).trim().to_owned(),
        })
    }
}

/// Keeps the first `cap` bytes and discards the rest while still draining the pipe, so a chatty
/// child can neither block on a full pipe nor grow our memory.
async fn read_capped(mut reader: impl AsyncRead + Unpin, cap: usize) -> Vec<u8> {
    let mut kept = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let room = cap.saturating_sub(kept.len());
                kept.extend_from_slice(&chunk[..n.min(room)]);
            }
        }
    }
    kept
}

/// Keeps only the last `cap` bytes: the end of ffmpeg's stderr is where the actual error is.
async fn read_tail(mut reader: impl AsyncRead + Unpin, cap: usize) -> Vec<u8> {
    let mut tail: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                tail.extend_from_slice(&chunk[..n]);
                if tail.len() > cap * 2 {
                    tail.drain(..tail.len() - cap);
                }
            }
        }
    }
    if tail.len() > cap {
        tail.drain(..tail.len() - cap);
    }
    tail
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_capped_reader_keeps_the_head_and_drains_the_rest() {
        let data = vec![7u8; 100_000];
        let kept = read_capped(&data[..], 1000).await;
        assert_eq!(kept.len(), 1000);
    }

    #[tokio::test]
    async fn the_tail_reader_keeps_the_end() {
        let mut data = vec![b'a'; 50_000];
        data.extend_from_slice(b"THE REAL ERROR");
        let tail = read_tail(&data[..], 100).await;
        assert_eq!(tail.len(), 100);
        assert!(tail.ends_with(b"THE REAL ERROR"));
    }

    #[tokio::test]
    async fn a_missing_binary_is_a_spawn_error() {
        let cancel = CancellationToken::new();
        let result = run(
            "definitely-not-a-real-binary-zeno",
            &[],
            None,
            Duration::from_secs(5),
            &cancel,
            10,
        )
        .await;
        assert!(matches!(result, Err(RunError::Spawn(_))));
    }
}
