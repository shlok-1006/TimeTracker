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
/// `manager_id = Some(id)` scopes to that manager's assigned users (via
/// `user_managers`) plus themselves; `None` (HR/admin) returns EVERYONE —
/// employees, project managers, and HR alike, so admin dashboards show every
/// person's live status. A stale heartbeat or missing row => `not_logged_in`.
pub async fn team(pool: &PgPool, manager_id: Option<Uuid>) -> Result<Vec<TeamMember>, AppError> {
    let rows = sqlx::query!(
        r#"
        SELECT u.id, u.name, u.email, u.role::text AS "role!",
               CASE WHEN p.last_seen_at IS NULL THEN 'not_logged_in'
                    WHEN EXTRACT(EPOCH FROM (now() - p.last_seen_at))::double precision > $2 THEN 'not_logged_in'
                    ELSE p.status::text END AS "status!",
               p.last_seen_at AS "last_seen_at?",
               -- Unioned, not summed (migration 0044): two devices signed in as the same person
               -- each record the same minute, and adding those up put someone's live card at
               -- nearly double their real day.
               CAST((SELECT s.active + s.meeting
                     FROM interval_seconds(u.id, date_trunc('day', now()), 'infinity'::timestamptz, NULL) s)
                    AS BIGINT) AS "today_seconds!"
        FROM users u
        LEFT JOIN presence p ON p.user_id = u.id
        WHERE ($1::uuid IS NULL OR u.id = $1
               OR EXISTS (SELECT 1 FROM user_managers um
                          WHERE um.user_id = u.id AND um.manager_id = $1))
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

/// Live status for the members of ONE team — the same rows as [`team`], scoped by
/// membership rather than by who manages whom.
///
/// The HRMS asked for this because the two disagreed: a PM could see a teammate's performance
/// score (team-scoped on their side) but not whether that person had shown up (manager-scoped on
/// ours), so the halves of one dashboard described different groups of people.
///
/// Membership only, deliberately: `team_pms` says who may ask, `user_teams` says who is listed.
/// A PM is not staff of their own team unless someone also put them on it.
pub async fn team_members(pool: &PgPool, team_id: Uuid) -> Result<Vec<TeamMember>, AppError> {
    let rows = sqlx::query!(
        r#"
        SELECT u.id, u.name, u.email, u.role::text AS "role!",
               CASE WHEN p.last_seen_at IS NULL THEN 'not_logged_in'
                    WHEN EXTRACT(EPOCH FROM (now() - p.last_seen_at))::double precision > $2 THEN 'not_logged_in'
                    ELSE p.status::text END AS "status!",
               p.last_seen_at AS "last_seen_at?",
               CAST((SELECT s.active + s.meeting
                     FROM interval_seconds(u.id, date_trunc('day', now()), 'infinity'::timestamptz, NULL) s)
                    AS BIGINT) AS "today_seconds!"
        FROM users u
        JOIN user_teams ut ON ut.user_id = u.id AND ut.team_id = $1
        LEFT JOIN presence p ON p.user_id = u.id
        ORDER BY u.name
        "#,
        team_id,
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
