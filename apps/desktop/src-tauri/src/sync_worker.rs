//! Background sync worker (Rule 4: SQLite -> sync queue -> API -> PostgreSQL).
//!
//! Periodically pushes pending intervals to `POST /intervals` via the
//! authenticated `http` helper (which refreshes the token on expiry). Never
//! blocks user actions; at-least-once delivery (server insert is idempotent).

use std::time::Duration;

use serde::Serialize;
use uuid::Uuid;

use crate::auth;
use crate::http;
use crate::interval_repository::{self, Interval};
use crate::timer::DesktopState;

const SYNC_INTERVAL: Duration = Duration::from_secs(15);
/// Max intervals per POST. A large offline backlog is drained in bounded chunks
/// so request bodies stay small and each insert is fast — one giant request
/// could time out or exceed the server's body limit and then never get acked.
const SYNC_CHUNK: usize = 500;

#[derive(Serialize)]
struct IntervalPayload {
    id: Uuid,
    start_utc: String,
    end_utc: String,
    kind: String,
    team_id: Option<Uuid>,
}

impl From<&Interval> for IntervalPayload {
    fn from(i: &Interval) -> Self {
        Self {
            id: i.id,
            start_utc: i.start_utc.to_rfc3339(),
            end_utc: i.end_utc.to_rfc3339(),
            kind: i.kind.clone(),
            team_id: i.team_id,
        }
    }
}

pub async fn run(state: DesktopState) {
    loop {
        tokio::time::sleep(SYNC_INTERVAL).await;
        if let Err(e) = sync_once(&state).await {
            tracing::warn!("interval sync failed (will retry): {e}");
        }
    }
}

async fn sync_once(state: &DesktopState) -> anyhow::Result<()> {
    if auth::stored_access().is_none() {
        return Ok(()); // not logged in
    }
    let pending = interval_repository::pending_sync(&state.pool).await?;
    if pending.is_empty() {
        return Ok(());
    }

    // Drain in bounded chunks. Each chunk is acked (marked synced) on its own,
    // so a large backlog makes steady progress and a failure only retries the
    // un-acked remainder rather than the whole pile.
    for chunk in pending.chunks(SYNC_CHUNK) {
        let payload: Vec<IntervalPayload> = chunk.iter().map(IntervalPayload::from).collect();
        http::post_json("/intervals", serde_json::to_value(&payload)?)
            .await
            .map_err(|e| anyhow::anyhow!(e))?;

        let ids: Vec<Uuid> = chunk.iter().map(|i| i.id).collect();
        interval_repository::mark_synced(&state.pool, &ids).await?;
        tracing::info!("synced {} interval(s)", ids.len());
    }
    Ok(())
}
