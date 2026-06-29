//! TimeTracker API server entrypoint.
//!
//! Boot sequence:
//!   1. Load `.env` and initialize tracing.
//!   2. Read configuration.
//!   3. Connect to PostgreSQL and run migrations.
//!   4. Build the router and serve.
//!
//! Errors propagate to `main` and abort startup (Rule 8: no unwrap in prod code).

use anyhow::Context;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use server::storage::{GcsConfig, StorageClient};
use server::{config::Config, db, jwt::JwtKeys, AppState};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Best-effort: a missing .env is fine (e.g. in production the env is injected).
    let _ = dotenvy::dotenv();
    init_tracing();

    let config = Config::from_env()?;
    tracing::info!(addr = %config.socket_addr, "starting TimeTracker API server");

    let pool = db::connect(&config.database_url, config.database_max_connections).await?;
    db::run_migrations(&pool).await?;
    tracing::info!("database connected and migrations applied");

    let jwt = JwtKeys::new(&config.jwt_access_secret, config.jwt_access_ttl_seconds);
    let storage = StorageClient::new(GcsConfig::from_env());
    let linear = server::linear_service::LinearService::from_env();
    tracing::info!(linear_configured = linear.is_configured(), "linear integration");
    let gemini = server::gemini_provider::GeminiProvider::from_env();
    tracing::info!(
        gemini_configured = gemini.is_configured(),
        model = gemini.model(),
        "vision AI provider"
    );
    let state = AppState::new(
        pool,
        jwt,
        storage,
        linear,
        gemini,
        config.jwt_refresh_ttl_seconds,
    );
    // Nightly analysis: builds the previous day's reports for every employee.
    tokio::spawn(server::analysis_scheduler::run(state.clone()));
    // Nightly attendance: rolls up the previous day's attendance for every employee.
    tokio::spawn(server::attendance_scheduler::run(state.clone()));

    let app = server::build_router(state);

    let listener = tokio::net::TcpListener::bind(config.socket_addr)
        .await
        .with_context(|| format!("failed to bind {}", config.socket_addr))?;

    tracing::info!(addr = %config.socket_addr, "listening");
    axum::serve(listener, app).await.context("server error")?;

    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,server=debug,sqlx=warn"));

    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .init();
}
