//! Transcode job runner: pops upload jobs off the queue, runs ffprobe + FFmpeg,
//! writes encrypted HLS renditions to object storage, marks the video ready.
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    tracing::info!("worker started, polling transcode queue");

    loop {
        // TODO: pop a job from Redis, run ffprobe + ffmpeg, package to HLS, mark video ready.
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
