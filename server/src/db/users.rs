//! Users repository (Rule 7: SQLx, compile-time checked queries, repository pattern).
//!
//! The Postgres `user_role` enum is crossed as `text` at the query boundary
//! (`role::text` on read, `$n::text::user_role` on write) so the macros resolve
//! to `String`, then we convert to the strongly-typed `UserRole` in Rust. This
//! keeps the queries compile-time checked without a bespoke enum type mapping.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::employment_type::EmploymentType;
use crate::error::AppError;
use crate::role::UserRole;

/// A user without secrets, for admin listing / management responses.
#[derive(Debug, Clone, Serialize)]
pub struct UserSummary {
    pub id: Uuid,
    pub name: String,
    pub email: String,
    pub role: UserRole,
    pub employment_type: EmploymentType,
    pub manager_id: Option<Uuid>,
    pub team_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct User {
    pub id: Uuid,
    pub name: String,
    pub email: String,
    pub password_hash: String,
    pub role: UserRole,
    pub employment_type: EmploymentType,
    pub manager_id: Option<Uuid>,
    pub team_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// NOTE: users.manager_id is deprecated — every scope/notification decision now
// reads the user_managers join table (an employee may have several managers).
// (The old manager_id_of() helper is gone with it.)

/// Store the user's IANA timezone (reported by the desktop), used only to
/// bucket the hours display at a 4 AM local boundary. Only accepts zones
/// Postgres recognizes — an unknown name would break `AT TIME ZONE` in
/// `hours_summary`, so it's ignored (the window falls back to UTC). Writes only
/// when the value actually changes (the heartbeat calls this every ~45s).
pub async fn set_timezone(pool: &PgPool, user_id: Uuid, tz: &str) -> Result<(), AppError> {
    if tz.is_empty() {
        return Ok(());
    }
    sqlx::query!(
        "UPDATE users SET timezone = $2, updated_at = now()
         WHERE id = $1 AND timezone IS DISTINCT FROM $2
           AND EXISTS (SELECT 1 FROM pg_timezone_names WHERE name = $2)",
        user_id,
        tz
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Is `manager` one of `user`'s managers? (PM scope checks — user_managers.)
pub async fn is_manager_of(pool: &PgPool, manager: Uuid, user: Uuid) -> Result<bool, AppError> {
    let row = sqlx::query!(
        r#"SELECT COUNT(*) AS "count!" FROM user_managers
           WHERE manager_id = $1 AND user_id = $2"#,
        manager,
        user
    )
    .fetch_one(pool)
    .await?;
    Ok(row.count > 0)
}

/// Identity of every manager assigned to a user (an employee can have several,
/// one, or none). Used to fan out PM notifications and for the manager editor.
pub async fn managers_of(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<Vec<(Uuid, String, String)>, AppError> {
    let rows = sqlx::query!(
        r#"
        SELECT u.id, u.name, u.email
        FROM user_managers um
        JOIN users u ON u.id = um.manager_id
        WHERE um.user_id = $1
        ORDER BY u.name
        "#,
        user_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| (r.id, r.name, r.email)).collect())
}

/// Add a single manager link (used on user creation). Idempotent.
pub async fn add_manager(pool: &PgPool, user_id: Uuid, manager_id: Uuid) -> Result<(), AppError> {
    sqlx::query!(
        "INSERT INTO user_managers (user_id, manager_id) VALUES ($1, $2)
         ON CONFLICT DO NOTHING",
        user_id,
        manager_id
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Replace a user's manager set atomically (empty = no managers).
pub async fn set_managers(
    pool: &PgPool,
    user_id: Uuid,
    manager_ids: &[Uuid],
) -> Result<(), AppError> {
    let mut tx = pool.begin().await?;
    sqlx::query!("DELETE FROM user_managers WHERE user_id = $1", user_id)
        .execute(&mut *tx)
        .await?;
    for mid in manager_ids {
        sqlx::query!(
            "INSERT INTO user_managers (user_id, manager_id) VALUES ($1, $2)
             ON CONFLICT DO NOTHING",
            user_id,
            mid
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

fn parse_role(s: &str) -> Result<UserRole, AppError> {
    s.parse::<UserRole>()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("invalid role stored in db: {e}")))
}

fn parse_employment_type(s: &str) -> Result<EmploymentType, AppError> {
    s.parse::<EmploymentType>().map_err(|e| {
        AppError::Internal(anyhow::anyhow!("invalid employment_type stored in db: {e}"))
    })
}

/// Look up a user by email. Returns `None` if no such user exists.
pub async fn find_by_email(pool: &PgPool, email: &str) -> Result<Option<User>, AppError> {
    let row = sqlx::query!(
        r#"
        SELECT id, name, email, password_hash, role::text AS "role!",
               employment_type::text AS "employment_type!",
               manager_id, team_id, created_at, updated_at
        FROM users
        WHERE email = $1
        "#,
        email
    )
    .fetch_optional(pool)
    .await?;

    match row {
        None => Ok(None),
        Some(r) => Ok(Some(User {
            id: r.id,
            name: r.name,
            email: r.email,
            password_hash: r.password_hash,
            role: parse_role(&r.role)?,
            employment_type: parse_employment_type(&r.employment_type)?,
            manager_id: r.manager_id,
            team_id: r.team_id,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })),
    }
}

/// Look up a user by id. Returns `None` if no such user exists.
pub async fn find_by_id(pool: &PgPool, id: Uuid) -> Result<Option<User>, AppError> {
    let row = sqlx::query!(
        r#"
        SELECT id, name, email, password_hash, role::text AS "role!",
               employment_type::text AS "employment_type!",
               manager_id, team_id, created_at, updated_at
        FROM users
        WHERE id = $1
        "#,
        id
    )
    .fetch_optional(pool)
    .await?;

    match row {
        None => Ok(None),
        Some(r) => Ok(Some(User {
            id: r.id,
            name: r.name,
            email: r.email,
            password_hash: r.password_hash,
            role: parse_role(&r.role)?,
            employment_type: parse_employment_type(&r.employment_type)?,
            manager_id: r.manager_id,
            team_id: r.team_id,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })),
    }
}

/// Insert a user, or update the existing one with the same email (idempotent).
/// Used by the dev seed.
pub async fn upsert(
    pool: &PgPool,
    name: &str,
    email: &str,
    password_hash: &str,
    role: UserRole,
    team_id: Option<Uuid>,
) -> Result<User, AppError> {
    let role_str = role.as_str();
    let r = sqlx::query!(
        r#"
        INSERT INTO users (name, email, password_hash, role, team_id)
        VALUES ($1, $2, $3, $4::text::user_role, $5)
        ON CONFLICT (email) DO UPDATE SET
            name = EXCLUDED.name,
            password_hash = EXCLUDED.password_hash,
            role = EXCLUDED.role,
            team_id = EXCLUDED.team_id,
            updated_at = now()
        RETURNING id, name, email, password_hash, role::text AS "role!",
                  employment_type::text AS "employment_type!",
                  manager_id, team_id, created_at, updated_at
        "#,
        name,
        email,
        password_hash,
        role_str,
        team_id
    )
    .fetch_one(pool)
    .await?;

    Ok(User {
        id: r.id,
        name: r.name,
        email: r.email,
        password_hash: r.password_hash,
        role: parse_role(&r.role)?,
        employment_type: parse_employment_type(&r.employment_type)?,
        manager_id: r.manager_id,
        team_id: r.team_id,
        created_at: r.created_at,
        updated_at: r.updated_at,
    })
}

/// List all users (admin management view).
pub async fn list_all(pool: &PgPool) -> Result<Vec<UserSummary>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT id, name, email, role::text AS "role!",
                  employment_type::text AS "employment_type!",
                  manager_id, team_id, created_at
           FROM users ORDER BY name"#
    )
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|r| {
            Ok(UserSummary {
                id: r.id,
                name: r.name,
                email: r.email,
                role: parse_role(&r.role)?,
                employment_type: parse_employment_type(&r.employment_type)?,
                manager_id: r.manager_id,
                team_id: r.team_id,
                created_at: r.created_at,
            })
        })
        .collect()
}

/// IDs of all employees (for batch jobs like the nightly attendance rollup).
pub async fn employee_ids(pool: &PgPool) -> Result<Vec<Uuid>, AppError> {
    let rows = sqlx::query!("SELECT id FROM users WHERE role = 'employee'::user_role")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|r| r.id).collect())
}

/// IDs to roll attendance up for on a given day window: every employee whose
/// account existed by then (attendance never predates the account), plus any
/// HR/PM who actually tracked time that day (they may use the desktop app
/// too). Portal-only admins are excluded so they don't accrue "absent" rows.
pub async fn attendance_rollup_ids(
    pool: &PgPool,
    day_start: chrono::DateTime<chrono::Utc>,
    day_end: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<Uuid>, AppError> {
    let rows = sqlx::query!(
        r#"
        SELECT id AS "id!" FROM users
        WHERE role = 'employee'::user_role AND created_at < $2
        UNION
        SELECT DISTINCT i.user_id AS "id!" FROM intervals i
        WHERE i.start_utc >= $1 AND i.start_utc < $2
        "#,
        day_start,
        day_end
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.id).collect())
}

/// `(name, email)` of every user with the given role — used to fan out
/// notifications (e.g. all HR recipients for the weekly hours warning).
pub async fn contacts_with_role(
    pool: &PgPool,
    role: UserRole,
) -> Result<Vec<(String, String)>, AppError> {
    let role_str = role.as_str();
    let rows = sqlx::query!(
        r#"SELECT name, email FROM users WHERE role = $1::text::user_role ORDER BY name"#,
        role_str
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| (r.name, r.email)).collect())
}

/// Create a new user. Returns `BadRequest` if the email already exists.
pub async fn create(
    pool: &PgPool,
    name: &str,
    email: &str,
    password_hash: &str,
    role: UserRole,
    manager_id: Option<Uuid>,
) -> Result<UserSummary, AppError> {
    let role_str = role.as_str();
    let result = sqlx::query!(
        r#"
        INSERT INTO users (name, email, password_hash, role, manager_id)
        VALUES ($1, $2, $3, $4::text::user_role, $5)
        RETURNING id, name, email, role::text AS "role!",
                  employment_type::text AS "employment_type!",
                  manager_id, team_id, created_at
        "#,
        name,
        email,
        password_hash,
        role_str,
        manager_id
    )
    .fetch_one(pool)
    .await;

    match result {
        Ok(r) => {
            // Manager assignment lives in user_managers (multi-manager capable);
            // the legacy users.manager_id written above is deprecated. Linking
            // here keeps every creation path (routes, seed, tests) consistent.
            if let Some(mid) = manager_id {
                if mid != r.id {
                    add_manager(pool, r.id, mid).await?;
                }
            }
            Ok(UserSummary {
                id: r.id,
                name: r.name,
                email: r.email,
                role: parse_role(&r.role)?,
                employment_type: parse_employment_type(&r.employment_type)?,
                manager_id: r.manager_id,
                team_id: r.team_id,
                created_at: r.created_at,
            })
        }
        Err(sqlx::Error::Database(db)) if db.is_unique_violation() => Err(AppError::BadRequest(
            "a user with that email already exists".into(),
        )),
        Err(e) => Err(e.into()),
    }
}

/// Delete a user (cascades intervals/presence/screenshots/etc). Returns whether
/// a row was removed.
pub async fn delete(pool: &PgPool, id: Uuid) -> Result<bool, AppError> {
    let res = sqlx::query!("DELETE FROM users WHERE id = $1", id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

/// Replace a user's password hash. Returns whether a row was updated.
pub async fn set_password(pool: &PgPool, id: Uuid, password_hash: &str) -> Result<bool, AppError> {
    let res = sqlx::query!(
        "UPDATE users SET password_hash = $2, updated_at = now() WHERE id = $1",
        id,
        password_hash
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Set a user's employment type (HR classification). Returns whether a row was
/// updated.
pub async fn set_employment_type(
    pool: &PgPool,
    id: Uuid,
    employment_type: EmploymentType,
) -> Result<bool, AppError> {
    let et = employment_type.as_str();
    let res = sqlx::query!(
        "UPDATE users SET employment_type = $2::text::employment_type, updated_at = now()
         WHERE id = $1",
        id,
        et
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// One directory entry for the cross-system identity handshake (HRMS integration).
/// `id` is the canonical UUID (== the JWT `sub`); `teams` are the team NAMES the
/// person belongs to (empty when none). Deliberately minimal — no secrets, no
/// internal fields — so it is safe for another system to read and reconcile against.
#[derive(Debug, Clone, Serialize)]
pub struct DirectoryEntry {
    pub id: Uuid,
    pub name: String,
    pub email: String,
    pub role: UserRole,
    pub teams: Vec<String>,
}

/// The canonical employee directory: every user with their team names, ordered by
/// name. One query with a `user_teams`→`teams` aggregation (LEFT JOIN so people on
/// no team still appear with an empty `teams`).
pub async fn list_directory(pool: &PgPool) -> Result<Vec<DirectoryEntry>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT u.id,
                  u.name,
                  u.email,
                  u.role::text AS "role!",
                  COALESCE(
                      array_agg(t.name ORDER BY t.name) FILTER (WHERE t.name IS NOT NULL),
                      ARRAY[]::text[]
                  ) AS "teams!"
           FROM users u
           LEFT JOIN user_teams ut ON ut.user_id = u.id
           LEFT JOIN teams t ON t.id = ut.team_id
           GROUP BY u.id, u.name, u.email, u.role
           ORDER BY u.name"#
    )
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|r| {
            Ok(DirectoryEntry {
                id: r.id,
                name: r.name,
                email: r.email,
                role: parse_role(&r.role)?,
                teams: r.teams,
            })
        })
        .collect()
}
