use tracing_subscriber::EnvFilter;
use zeno_keyserver::{router, KeyLimiter, KeyServer, KeyServerConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    if std::env::var("LOG_FORMAT").is_ok_and(|v| v.eq_ignore_ascii_case("json")) {
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter)
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }

    let config = KeyServerConfig::from_env()?;
    // Only reads keys and touches sessions: a small pool is plenty.
    let db = sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&config.database_url)
        .await?;
    let redis = redis::Client::open(config.redis_url.as_str())?;
    let redis = redis::aio::ConnectionManager::new(redis).await?;
    let limiter = KeyLimiter::redis(redis, config.ip_limit_per_minute, config.user_limit_per_day);
    let app = router(KeyServer::new(db, &config, limiter)?);

    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    tracing::info!(env = ?config.env, "keyserver listening on {}", config.bind_addr);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    Ok(())
}
