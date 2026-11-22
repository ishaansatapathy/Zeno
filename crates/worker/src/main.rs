use std::sync::Arc;

use anyhow::Context as _;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;
use zeno_core::storage::S3Store;
use zeno_worker::config::WorkerConfig;
use zeno_worker::pipeline::Context;
use zeno_worker::runner::Lane;
use zeno_worker::{preflight, runner};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    let logs = tracing_subscriber::fmt().with_env_filter(
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn")),
    );
    if std::env::var("LOG_FORMAT").is_ok_and(|v| v.eq_ignore_ascii_case("json")) {
        logs.json().init();
    } else {
        logs.init();
    }

    let config = WorkerConfig::from_env().context("invalid configuration")?;
    // A connection each for the runners, one that listens for announcements, and a few spare.
    let pool_size = u32::try_from(config.concurrency + config.rest_concurrency).unwrap_or(2) + 4;
    let db = zeno_core::db::connect(&config.database_url, pool_size)
        .await
        .context("could not connect to Postgres")?;
    let store = S3Store::new(&config.storage).context("invalid storage configuration")?;
    let (concurrency, rest_concurrency) = (config.concurrency, config.rest_concurrency);
    let ctx = Context { db, store, config };

    preflight::check(&ctx)
        .await
        .context("encoder is not usable")?;
    preflight::remove_stale_work_dirs(&ctx.config.work_dir).await;

    let shutdown = CancellationToken::new();
    tokio::spawn({
        let shutdown = shutdown.clone();
        async move {
            wait_for_signal().await;
            tracing::info!("shutting down; releasing the current job");
            shutdown.cancel();
        }
    });

    tracing::info!(concurrency, rest_concurrency, "worker started");
    let wake = Arc::new(Notify::new());
    tokio::spawn(runner::listen_for_jobs(
        ctx.db.clone(),
        wake.clone(),
        shutdown.clone(),
    ));
    let mut runners = Vec::new();
    for (lane, count) in [(Lane::First, concurrency), (Lane::Rest, rest_concurrency)] {
        for _ in 0..count {
            runners.push(tokio::spawn(runner::run_lane(
                ctx.clone(),
                shutdown.clone(),
                lane,
                wake.clone(),
                runner::IDLE_POLL,
            )));
        }
    }
    for runner in runners {
        let _ = runner.await;
    }
    Ok(())
}

async fn wait_for_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
