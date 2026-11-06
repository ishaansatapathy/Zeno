use anyhow::Context as _;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;
use zeno_core::storage::S3Store;
use zeno_worker::config::WorkerConfig;
use zeno_worker::pipeline::Context;
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
    let pool_size = u32::try_from(config.concurrency).unwrap_or(1) + 3;
    let db = zeno_core::db::connect(&config.database_url, pool_size)
        .await
        .context("could not connect to Postgres")?;
    let store = S3Store::new(&config.storage).context("invalid storage configuration")?;
    let concurrency = config.concurrency;
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

    tracing::info!(concurrency, "worker started");
    let mut runners = Vec::new();
    for _ in 0..concurrency {
        runners.push(tokio::spawn(runner::run_forever(
            ctx.clone(),
            shutdown.clone(),
        )));
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
