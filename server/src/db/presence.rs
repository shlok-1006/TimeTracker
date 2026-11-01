//! Presence repository (Rule 7: compile-time checked queries).

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;
use crate::org_time;
use crate::presence::{roster_status, PresenceStatus};

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

/// Rows for devices not heard from in this long are dropped on the owner's next heartbeat.
/// Far past any grace window; it only stops reinstalled machines piling up.
pub const DEVICE_RETENTION_SECONDS: f64 = 86_400.0;

/// Which `presence_devices` row a heartbeat lands on (migration 0058).
///
/// A build that sends its install id gets a row of its own. An older build sends none, so its
/// beats are keyed by the status they report: two old-build machines stay apart whenever they
/// disagree, which is all the badge needs.
pub fn device_key(device_id: Option<Uuid>, status: PresenceStatus) -> String {
    match device_id {
        Some(id) => format!("device:{}", id.simple()),
        None => format!("legacy:{}", status.as_str()),
    }
}

/// Record a heartbeat for `user_id` from one of their devices.
///
/// `presence` keeps the last heartbeat as before; `presence_devices` keeps one row per device,
/// which is what the live status is derived from.
pub async fn heartbeat(
    pool: &PgPool,
    user_id: Uuid,
    status: PresenceStatus,
    current_interval_id: Option<Uuid>,
    device_id: Option<Uuid>,
) -> Result<(), AppError> {
    let status_str = status.as_str();
    let key = device_key(device_id, status);
    let mut tx = pool.begin().await?;
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
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        r#"
        INSERT INTO presence_devices (user_id, device_key, status, last_seen_at)
        VALUES ($1, $2, $3::text::presence_status, now())
        ON CONFLICT (user_id, device_key) DO UPDATE SET
            status = EXCLUDED.status,
            last_seen_at = now()
        "#,
        user_id,
        key,
        status_str
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        r#"
        DELETE FROM presence_devices
        WHERE user_id = $1
          AND device_key <> $2
          AND last_seen_at < now() - make_interval(secs => $3)
        "#,
        user_id,
        key,
        DEVICE_RETENTION_SECONDS
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// The live status shown for `user_id` right now: the best status among their devices heard
/// from within [`GRACE_SECONDS`], or `not_logged_in`.
pub async fn live_status(pool: &PgPool, user_id: Uuid) -> Result<String, AppError> {
    let status = sqlx::query_scalar!(
        r#"SELECT presence_live_status($1, $2) AS "status!""#,
        user_id,
        GRACE_SECONDS
    )
    .fetch_one(pool)
    .await?;
    Ok(status)
}

/// Team roster with live, server-derived statuses.
///
/// `manager_id = Some(id)` scopes to that manager's assigned users (via
/// `user_managers`) plus themselves; `None` (HR/admin) returns EVERYONE —
/// employees, project managers, and HR alike, so admin dashboards show every
/// person's live status. A stale heartbeat or missing row => `not_logged_in`, then
/// [`roster_status`] turns "off the clock" into `on_leave` (approved leave today),
/// `logged_out` (already worked today) or `not_logged_in` (app open, timer never started).
pub async fn team(pool: &PgPool, manager_id: Option<Uuid>) -> Result<Vec<TeamMember>, AppError> {
    let rows = sqlx::query!(
        r#"
        SELECT u.id, u.name, u.email, u.role::text AS "role!",
               -- Best status across the person's devices (migration 0058): one machine on a
               -- break while another is working reads as working, not a flip-flop between them.
               presence_live_status(u.id, $2) AS "status!",
               p.last_seen_at AS "last_seen_at?",
               -- Unioned, not summed (migration 0044): two devices signed in as the same person
               -- each record the same minute, and adding those up put someone's live card at
               -- nearly double their real day.
               CAST((SELECT s.active + s.meeting
                     FROM interval_seconds(u.id, date_trunc('day', now()), 'infinity'::timestamptz, NULL) s)
                    AS BIGINT) AS "today_seconds!",
               -- An approved leave covering today (org day, Asia/Kolkata) → "on_leave" off the clock.
               EXISTS (SELECT 1 FROM leave_requests lr
                       WHERE lr.user_id = u.id AND lr.status = 'approved'
                         AND lr.start_date <= $3 AND lr.end_date >= $3) AS "on_leave_today!"
        FROM users u
        LEFT JOIN presence p ON p.user_id = u.id
        -- A deactivated account is off the roster everywhere else (directory, Manage users,
        -- attendance); the live board must drop them too, or a leaver lingers here forever.
        WHERE u.deactivated_at IS NULL
          AND ($1::uuid IS NULL OR u.id = $1
               OR EXISTS (SELECT 1 FROM user_managers um
                          WHERE um.user_id = u.id AND um.manager_id = $1))
        ORDER BY u.name
        "#,
        manager_id,
        GRACE_SECONDS,
        org_time::today()
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
            status: roster_status(&r.status, r.today_seconds > 0, r.on_leave_today),
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
               presence_live_status(u.id, $2) AS "status!",
               p.last_seen_at AS "last_seen_at?",
               CAST((SELECT s.active + s.meeting
                     FROM interval_seconds(u.id, date_trunc('day', now()), 'infinity'::timestamptz, NULL) s)
                    AS BIGINT) AS "today_seconds!",
               -- An approved leave covering today (org day, Asia/Kolkata) → "on_leave" off the clock.
               EXISTS (SELECT 1 FROM leave_requests lr
                       WHERE lr.user_id = u.id AND lr.status = 'approved'
                         AND lr.start_date <= $3 AND lr.end_date >= $3) AS "on_leave_today!"
        FROM users u
        JOIN user_teams ut ON ut.user_id = u.id AND ut.team_id = $1
        LEFT JOIN presence p ON p.user_id = u.id
        WHERE u.deactivated_at IS NULL
        ORDER BY u.name
        "#,
        team_id,
        GRACE_SECONDS,
        org_time::today()
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
            status: roster_status(&r.status, r.today_seconds > 0, r.on_leave_today),
            last_seen_at: r.last_seen_at,
            today_seconds: r.today_seconds,
        })
        .collect())
}
