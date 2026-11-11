use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{bail, Context};
use tracing_subscriber::EnvFilter;
use zeno_api::auth::counters::RedisCounters;
use zeno_api::auth::password::{PasswordParams, Passwords};
use zeno_api::config::ApiConfig;
use zeno_api::mail::LogMailer;
use zeno_api::state::AppState;
use zeno_core::storage::S3Store;

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

    let config = ApiConfig::from_env().context("invalid configuration")?;
    if config.core.env.is_production() {
        bail!("no production mail transport is configured yet; refusing to start in production");
    }

    if config.payments.uses_test_keys() && config.core.env.is_production() {
        tracing::warn!("production is configured with Razorpay test keys: no real money will move");
    }

    let db = zeno_core::db::connect(&config.core.database_url, 10)
        .await
        .context("could not connect to Postgres")?;
    if !config.core.env.is_production() {
        zeno_core::db::MIGRATOR
            .run(&db)
            .await
            .context("running migrations")?;
    }

    let redis = redis::Client::open(config.core.redis_url.as_str()).context("invalid REDIS_URL")?;
    let redis = redis::aio::ConnectionManager::new(redis)
        .await
        .context("could not connect to Redis")?;

    let store = S3Store::new(&config.storage).context("invalid storage configuration")?;
    if !config.core.env.is_production() {
        store
            .ensure_bucket()
            .await
            .context("could not reach object storage")?;
    }

    let bind_addr = config.core.bind_addr;
    let state = AppState::new(
        db,
        config,
        Arc::new(RedisCounters::new(redis.clone())),
        Arc::new(LogMailer),
        Passwords::new(PasswordParams::default())?,
        store,
    )
    .with_limiter(zeno_core::ratelimit::RateLimiter::redis(redis));

    tracing::info!(provider = state.payments.name(), "payments ready");
    zeno_api::auth::cleanup::spawn_periodic(state.clone());

    zeno_api::outbox::spawn_retry_loop(state.clone());
    zeno_api::ledger::integrity::spawn_daily(state.clone());
    zeno_api::reconcile::spawn_periodic(
        state.clone(),
        std::time::Duration::from_secs(state.config.reconcile_interval_secs),
    );
    let background = state.background.clone();
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    tracing::info!("api listening on {bind_addr}");
    axum::serve(
        listener,
        zeno_api::build_app(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    // Let queued emails and follow-up jobs finish, but never hang shutdown on them.
    let _ = tokio::time::timeout(std::time::Duration::from_secs(10), background.idle()).await;
    Ok(())
}

async fn shutdown_signal() {
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
    tracing::info!("shutting down");
}
