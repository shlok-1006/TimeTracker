//! Presence repository (Rule 7: compile-time checked queries).

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;
use crate::presence::PresenceStatus;

/// Grace period (seconds): if the last heartbeat is older than this, the user is
/// derived as `not_logged_in`. The desktop beats every ~45s, so 2× gives slack.
pub const GRACE_SECONDS: f64 = 90.0;

/// A team member's derived live status.
#[derive(Debug)]
pub struct TeamMember {
    pub id: Uuid,
    pub name: String,
    pub email: String,
    pub role: String,
    pub status: String,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub today_seconds: i64,
}

/// Upsert the heartbeat for `user_id`.
pub async fn heartbeat(
    pool: &PgPool,
    user_id: Uuid,
    status: PresenceStatus,
    current_interval_id: Option<Uuid>,
) -> Result<(), AppError> {
    let status_str = status.as_str();
    sqlx::query!(
        r#"
        INSERT INTO presence (user_id, status, last_seen_at, current_interval_id)
        VALUES ($1, $2::text::presence_status, now(), $3)
        ON CONFLICT (user_id) DO UPDATE SET
            status = EXCLUDED.status,
            last_seen_at = now(),
            current_interval_id = EXCLUDED.current_interval_id
        "#,
        user_id,
        status_str,
        current_interval_id
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Team roster with live, server-derived statuses.
///
/// `manager_id = Some(id)` scopes to that manager's team; `None` (HR/admin)
/// returns all employees. A stale heartbeat or missing row => `not_logged_in`.
pub async fn team(pool: &PgPool, manager_id: Option<Uuid>) -> Result<Vec<TeamMember>, AppError> {
    let rows = sqlx::query!(
        r#"
        SELECT u.id, u.name, u.email, u.role::text AS "role!",
               CASE WHEN p.last_seen_at IS NULL THEN 'not_logged_in'
                    WHEN EXTRACT(EPOCH FROM (now() - p.last_seen_at))::double precision > $2 THEN 'not_logged_in'
                    ELSE p.status::text END AS "status!",
               p.last_seen_at AS "last_seen_at?",
               CAST(COALESCE((SELECT SUM(EXTRACT(EPOCH FROM (i.end_utc - i.start_utc)))
                              FROM intervals i
                              WHERE i.user_id = u.id AND i.kind IN ('active','meeting')
                                AND i.start_utc >= date_trunc('day', now())), 0) AS BIGINT) AS "today_seconds!"
        FROM users u
        LEFT JOIN presence p ON p.user_id = u.id
        WHERE u.role = 'employee' AND ($1::uuid IS NULL OR u.manager_id = $1)
        ORDER BY u.name
        "#,
        manager_id,
        GRACE_SECONDS
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| TeamMember {
            id: r.id,
            name: r.name,
            email: r.email,
            role: r.role,
            status: r.status,
            last_seen_at: r.last_seen_at,
            today_seconds: r.today_seconds,
        })
        .collect())
}
