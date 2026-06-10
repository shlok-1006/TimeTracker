//! Audit log (append-only; the table blocks UPDATE/DELETE via trigger).
//! Records sensitive actions like user creation/deletion (CLAUDE.md).

use sqlx::PgPool;
use uuid::Uuid;

pub async fn log(
    pool: &PgPool,
    actor_id: Uuid,
    action: &str,
    entity_type: &str,
    entity_id: Option<Uuid>,
) {
    // Best-effort: an audit failure must not block the action.
    if let Err(e) = sqlx::query!(
        "INSERT INTO audit_logs (actor_id, action, entity_type, entity_id) VALUES ($1, $2, $3, $4)",
        actor_id,
        action,
        entity_type,
        entity_id
    )
    .execute(pool)
    .await
    {
        tracing::warn!("failed to write audit log ({action}): {e}");
    }
}
