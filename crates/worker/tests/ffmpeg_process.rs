mod common;

use std::time::{Duration, Instant};

use common::ffmpeg_bin;
use tokio_util::sync::CancellationToken;
use zeno_worker::ffmpeg::{run, RunError};

/// An ffmpeg run that lasts a full minute of wall-clock time (`-re` paces it in real time),
/// so any early return must have come from us stopping it.
fn one_minute_job() -> Vec<String> {
    [
        "-nostdin",
        "-v",
        "error",
        "-re",
        "-f",
        "lavfi",
        "-i",
        "testsrc=size=160x120:rate=10:duration=60",
        "-f",
        "null",
        "-",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect()
}

#[tokio::test]
async fn cancelling_stops_a_running_encoder_immediately() {
    let cancel = CancellationToken::new();
    let canceller = {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(600)).await;
            cancel.cancel();
        })
    };
    let started = Instant::now();
    let result = run(
        &ffmpeg_bin(),
        &one_minute_job(),
        None,
        Duration::from_secs(120),
        &cancel,
        0,
    )
    .await;
    canceller.await.unwrap();

    assert!(matches!(result, Err(RunError::Cancelled)), "{result:?}");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "took {:?}; the process was not killed",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_runaway_encoder_is_killed_when_its_timeout_expires() {
    let started = Instant::now();
    let result = run(
        &ffmpeg_bin(),
        &one_minute_job(),
        None,
        Duration::from_millis(800),
        &CancellationToken::new(),
        0,
    )
    .await;
    assert!(matches!(result, Err(RunError::Timeout)), "{result:?}");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "took {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn failures_report_the_end_of_stderr_and_successes_return_stdout() {
    let cancel = CancellationToken::new();
    let bad = run(
        &ffmpeg_bin(),
        &[
            "-nostdin".into(),
            "-v".into(),
            "error".into(),
            "-i".into(),
            "file:does-not-exist.mp4".into(),
            "-f".into(),
            "null".into(),
            "-".into(),
        ],
        None,
        Duration::from_secs(30),
        &cancel,
        0,
    )
    .await;
    match bad {
        Err(RunError::Failed { stderr_tail, .. }) => {
            assert!(stderr_tail.contains("does-not-exist"), "{stderr_tail}")
        }
        other => panic!("{other:?}"),
    }

    let ok = run(
        &ffmpeg_bin(),
        &["-version".into()],
        None,
        Duration::from_secs(30),
        &cancel,
        1000,
    )
    .await
    .unwrap();
    assert!(String::from_utf8_lossy(&ok.stdout).starts_with("ffmpeg version"));
    assert!(ok.stdout.len() <= 1000, "stdout is capped");
}

#[tokio::test]
async fn the_child_does_not_inherit_the_workers_secrets() {
    std::env::set_var("ZENO_TEST_SECRET", "super-secret-value");
    let cancel = CancellationToken::new();
    // `-version` prints the environment-independent banner; to observe the environment we use a
    // process that echoes it. ffmpeg cannot, so check the platform shell-free tool instead.
    let (bin, args): (&str, Vec<String>) = if cfg!(windows) {
        ("cmd", vec!["/C".into(), "set".into()])
    } else {
        ("env", vec![])
    };
    let output = run(bin, &args, None, Duration::from_secs(30), &cancel, 1 << 20)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        !text.contains("super-secret-value"),
        "the environment must be scrubbed"
    );
}
