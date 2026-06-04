//! Background sync worker (Rule 4: SQLite -> sync queue -> API -> PostgreSQL).
//!
//! Periodically pushes pending intervals to `POST /intervals`. It never blocks
//! user actions (Rule 1): failures are logged and retried on the next tick, and
//! delivery is at-least-once (the server insert is idempotent).

use std::time::Duration;

use serde::Serialize;
use uuid::Uuid;

use crate::auth;
use crate::interval_repository::{self, Interval};
use crate::timer::DesktopState;

const SYNC_INTERVAL: Duration = Duration::from_secs(15);

fn api_base() -> String {
    std::env::var("TIMETRACKER_API_BASE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string())
}

/// Wire payload — `user_id` is intentionally omitted (server derives it).
#[derive(Serialize)]
struct IntervalPayload {
    id: Uuid,
    start_utc: String,
    end_utc: String,
    idle: bool,
}

impl From<&Interval> for IntervalPayload {
    fn from(i: &Interval) -> Self {
        Self {
            id: i.id,
            start_utc: i.start_utc.to_rfc3339(),
            end_utc: i.end_utc.to_rfc3339(),
            idle: i.idle,
        }
    }
}

/// Run forever, syncing pending intervals on a fixed interval.
pub async fn run(state: DesktopState) {
    let client = reqwest::Client::new();
    loop {
        tokio::time::sleep(SYNC_INTERVAL).await;
        if let Err(e) = sync_once(&client, &state).await {
            tracing::warn!("interval sync failed (will retry): {e}");
        }
    }
}

async fn sync_once(client: &reqwest::Client, state: &DesktopState) -> anyhow::Result<()> {
    // No token => not logged in yet; nothing to do.
    let token = match auth::stored_token() {
        Some(t) => t,
        None => return Ok(()),
    };

    let pending = interval_repository::pending_sync(&state.pool).await?;
    if pending.is_empty() {
        return Ok(());
    }

    let payload: Vec<IntervalPayload> = pending.iter().map(IntervalPayload::from).collect();
    let resp = client
        .post(format!("{}/intervals", api_base()))
        .bearer_auth(token)
        .json(&payload)
        .send()
        .await?;

    if !resp.status().is_success() {
        anyhow::bail!("server returned {}", resp.status());
    }

    let ids: Vec<Uuid> = pending.iter().map(|i| i.id).collect();
    interval_repository::mark_synced(&state.pool, &ids).await?;
    tracing::info!("synced {} interval(s)", ids.len());
    Ok(())
}
