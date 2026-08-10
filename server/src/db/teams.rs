//! Teams repository (Feature 4, Rule 7): team catalogue + multi-team membership.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Clone, Serialize)]
pub struct Team {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub created_at: DateTime<Utc>,
}

/// A team member (an employee belonging to a team).
#[derive(Debug, Clone, Serialize)]
pub struct TeamMember {
    pub id: Uuid,
    pub name: String,
    pub email: String,
}

// ---- Teams ----

pub async fn create(pool: &PgPool, name: &str, description: &str) -> Result<Team, AppError> {
    let result = sqlx::query!(
        "INSERT INTO teams (name, description) VALUES ($1, $2)
         RETURNING id, name, description, created_at",
        name,
        description
    )
    .fetch_one(pool)
    .await;

    match result {
        Ok(r) => Ok(Team {
            id: r.id,
            name: r.name,
            description: r.description,
            created_at: r.created_at,
        }),
        Err(sqlx::Error::Database(db)) if db.is_unique_violation() => Err(AppError::BadRequest(
            "a team with that name already exists".into(),
        )),
        Err(e) => Err(e.into()),
    }
}

pub async fn list(pool: &PgPool) -> Result<Vec<Team>, AppError> {
    let rows = sqlx::query!("SELECT id, name, description, created_at FROM teams ORDER BY name")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| Team {
            id: r.id,
            name: r.name,
            description: r.description,
            created_at: r.created_at,
        })
        .collect())
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Team>, AppError> {
    let row = sqlx::query!(
        "SELECT id, name, description, created_at FROM teams WHERE id = $1",
        id
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| Team {
        id: r.id,
        name: r.name,
        description: r.description,
        created_at: r.created_at,
    }))
}

/// Update a team's name and/or description (PATCH semantics: `None` fields are
/// left unchanged). Returns `None` if no team has that id.
pub async fn update(
    pool: &PgPool,
    id: Uuid,
    name: Option<&str>,
    description: Option<&str>,
) -> Result<Option<Team>, AppError> {
    let result = sqlx::query!(
        r#"UPDATE teams
           SET name = COALESCE($2, name), description = COALESCE($3, description)
           WHERE id = $1
           RETURNING id, name, description, created_at"#,
        id,
        name,
        description
    )
    .fetch_optional(pool)
    .await;

    match result {
        Ok(row) => Ok(row.map(|r| Team {
            id: r.id,
            name: r.name,
            description: r.description,
            created_at: r.created_at,
        })),
        Err(sqlx::Error::Database(db)) if db.is_unique_violation() => Err(AppError::BadRequest(
            "a team with that name already exists".into(),
        )),
        Err(e) => Err(e.into()),
    }
}

/// Delete a team. Membership rows and interval `team_id`s are cleaned up by the
/// FK rules (cascade / set null). Returns whether a row was removed.
pub async fn delete(pool: &PgPool, id: Uuid) -> Result<bool, AppError> {
    let res = sqlx::query!("DELETE FROM teams WHERE id = $1", id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

// ---- Membership ----

/// Add an employee to a team (idempotent). Returns Err if the team doesn't exist.
pub async fn add_member(pool: &PgPool, user_id: Uuid, team_id: Uuid) -> Result<(), AppError> {
    let res = sqlx::query!(
        "INSERT INTO user_teams (user_id, team_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        user_id,
        team_id
    )
    .execute(pool)
    .await;

    match res {
        Ok(_) => Ok(()),
        Err(sqlx::Error::Database(db)) if db.is_foreign_key_violation() => {
            Err(AppError::BadRequest("unknown user or team".into()))
        }
        Err(e) => Err(e.into()),
    }
}

/// Remove an employee from a team. Returns whether a membership was removed.
pub async fn remove_member(pool: &PgPool, user_id: Uuid, team_id: Uuid) -> Result<bool, AppError> {
    let res = sqlx::query!(
        "DELETE FROM user_teams WHERE user_id = $1 AND team_id = $2",
        user_id,
        team_id
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// The teams an employee belongs to (used by the desktop's pre-timer dropdown).
pub async fn teams_for_user(pool: &PgPool, user_id: Uuid) -> Result<Vec<Team>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT t.id, t.name, t.description, t.created_at
           FROM teams t
           JOIN user_teams ut ON ut.team_id = t.id
           WHERE ut.user_id = $1
           ORDER BY t.name"#,
        user_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Team {
            id: r.id,
            name: r.name,
            description: r.description,
            created_at: r.created_at,
        })
        .collect())
}

/// The employees in a team.
pub async fn members_of(pool: &PgPool, team_id: Uuid) -> Result<Vec<TeamMember>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT u.id, u.name, u.email
           FROM users u
           JOIN user_teams ut ON ut.user_id = u.id
           WHERE ut.team_id = $1
           ORDER BY u.name"#,
        team_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| TeamMember {
            id: r.id,
            name: r.name,
            email: r.email,
        })
        .collect())
}

// ---- Summary metrics (Feature 4 Phase 4) ----

/// A team plus its member count, for the team index.
#[derive(Debug, Clone, Serialize)]
pub struct TeamWithCount {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub created_at: DateTime<Utc>,
    pub member_count: i64,
}

/// Worked-time status breakdown for a team (seconds). `total` = active + meeting.
#[derive(Debug, Clone, Serialize)]
pub struct StatusBreakdown {
    pub total: i64,
    pub active: i64,
    pub idle: i64,
    pub meeting: i64,
    pub break_: i64,
}

/// One member's worked total within a team (seconds).
#[derive(Debug, Clone, Serialize)]
pub struct MemberTotal {
    pub user_id: Uuid,
    pub name: String,
    pub email: String,
    pub worked_seconds: i64,
}

/// All teams with member counts. `manager_id = Some(pm)` scopes counts to that
/// PM's managed employees and hides teams none of them belong to; `None` (HR)
/// returns every team with its full member count (SEC-09).
pub async fn list_with_counts(
    pool: &PgPool,
    manager_id: Option<Uuid>,
) -> Result<Vec<TeamWithCount>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT t.id, t.name, t.description, t.created_at,
                  CAST(COUNT(u.id) FILTER (WHERE $1::uuid IS NULL
                       OR EXISTS (SELECT 1 FROM user_managers um
                                  WHERE um.user_id = u.id AND um.manager_id = $1)) AS BIGINT) AS "member_count!"
           FROM teams t
           LEFT JOIN user_teams ut ON ut.team_id = t.id
           LEFT JOIN users u ON u.id = ut.user_id
           GROUP BY t.id, t.name, t.description, t.created_at
           HAVING $1::uuid IS NULL
               OR COUNT(u.id) FILTER (WHERE EXISTS (SELECT 1 FROM user_managers um
                                                    WHERE um.user_id = u.id AND um.manager_id = $1)) > 0
           ORDER BY t.name"#,
        manager_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| TeamWithCount {
            id: r.id,
            name: r.name,
            description: r.description,
            created_at: r.created_at,
            member_count: r.member_count,
        })
        .collect())
}

/// Team worked-time status breakdown (over `intervals.team_id`). `manager_id =
/// Some(pm)` limits it to that PM's managed employees; `None` (HR) is team-wide.
pub async fn status_breakdown(
    pool: &PgPool,
    team_id: Uuid,
    manager_id: Option<Uuid>,
) -> Result<StatusBreakdown, AppError> {
    let r = sqlx::query!(
        // Per member, then added up: overlap is a per-person problem (one person's two
        // devices), so unioning across the whole team would wrongly merge two different
        // people working the same hour.
        r#"WITH per_member AS (
             SELECT s.*
             FROM users u
             JOIN user_teams ut ON ut.user_id = u.id AND ut.team_id = $1
             CROSS JOIN LATERAL interval_seconds(
               u.id, '-infinity'::timestamptz, 'infinity'::timestamptz, $1) s
             WHERE $2::uuid IS NULL
                OR EXISTS (SELECT 1 FROM user_managers um
                           WHERE um.user_id = u.id AND um.manager_id = $2)
           )
           SELECT
             CAST(COALESCE(SUM(active + meeting),0) AS BIGINT) AS "total!",
             CAST(COALESCE(SUM(active),0)  AS BIGINT) AS "active!",
             CAST(COALESCE(SUM(idle),0)    AS BIGINT) AS "idle!",
             CAST(COALESCE(SUM(meeting),0) AS BIGINT) AS "meeting!",
             CAST(COALESCE(SUM(brk),0)     AS BIGINT) AS "brk!"
           FROM per_member"#,
        team_id,
        manager_id
    )
    .fetch_one(pool)
    .await?;
    Ok(StatusBreakdown {
        total: r.total,
        active: r.active,
        idle: r.idle,
        meeting: r.meeting,
        break_: r.brk,
    })
}

/// Per-member worked totals within a team. `manager_id = Some(pm)` returns only
/// that PM's managed employees; `None` (HR) returns every member (SEC-09).
pub async fn member_totals(
    pool: &PgPool,
    team_id: Uuid,
    manager_id: Option<Uuid>,
) -> Result<Vec<MemberTotal>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT u.id, u.name, u.email,
                  CAST(s.active + s.meeting AS BIGINT) AS "worked!"
           FROM users u
           JOIN user_teams ut ON ut.user_id = u.id AND ut.team_id = $1
           CROSS JOIN LATERAL interval_seconds(
             u.id, '-infinity'::timestamptz, 'infinity'::timestamptz, $1) s
           WHERE $2::uuid IS NULL
              OR EXISTS (SELECT 1 FROM user_managers um
                         WHERE um.user_id = u.id AND um.manager_id = $2)
           ORDER BY 4 DESC, u.name"#,
        team_id,
        manager_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| MemberTotal {
            user_id: r.id,
            name: r.name,
            email: r.email,
            worked_seconds: r.worked,
        })
        .collect())
}

/// Whether an employee belongs to a team (validates timer team selection).
pub async fn is_member(pool: &PgPool, user_id: Uuid, team_id: Uuid) -> Result<bool, AppError> {
    let row = sqlx::query!(
        r#"SELECT EXISTS(
              SELECT 1 FROM user_teams WHERE user_id = $1 AND team_id = $2
           ) AS "member!""#,
        user_id,
        team_id
    )
    .fetch_one(pool)
    .await?;
    Ok(row.member)
}

/// A member of a team, as the roster needs them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMemberRef {
    pub user_id: Uuid,
    pub name: String,
    pub email: String,
    pub employee_code: Option<String>,
}

/// A team with its roster and the managers who cover it.
#[derive(Debug, Clone, Serialize)]
pub struct TeamDetail {
    pub team_id: Uuid,
    pub name: String,
    pub description: String,
    pub member_count: i64,
    pub members: Vec<TeamMemberRef>,
    /// The project managers assigned to this team (`team_pms`, migration 0045).
    ///
    /// This used to be DERIVED — the managers of the team's members — because management was only
    /// ever a person-to-person relation here. That could not express a PM who manages nobody on
    /// the team, and let a member with two managers contribute both to a PM set nobody chose.
    /// It is now declared, and the shape is unchanged: the migration seeds the table with exactly
    /// what the derivation used to return, so this field reads the same on day one.
    pub pms: Vec<TeamMemberRef>,
    pub created_at: DateTime<Utc>,
}

/// Every team with its members and PMs, in ONE query.
///
/// Callers previously had to list teams, then fetch each team's members separately — an N+1 that
/// made rendering a roster (or "the PM's team") cost a request per team. Scope matches
/// `list_with_counts`: HR sees every team in full; a PM sees only teams they manage someone on,
/// and only the members they actually manage.
pub async fn list_detailed(
    pool: &PgPool,
    manager_id: Option<Uuid>,
) -> Result<Vec<TeamDetail>, AppError> {
    let rows = sqlx::query!(
        r#"WITH visible AS (
               SELECT ut.team_id, u.id AS user_id, u.name, u.email, u.employee_code
               FROM user_teams ut
               JOIN users u ON u.id = ut.user_id
               WHERE $1::uuid IS NULL
                  OR EXISTS (SELECT 1 FROM user_managers um
                             WHERE um.user_id = u.id AND um.manager_id = $1)
           )
           SELECT t.id, t.name, t.description, t.created_at,
                  CAST(COUNT(v.user_id) AS BIGINT) AS "member_count!",
                  COALESCE(
                      json_agg(json_build_object(
                          'user_id', v.user_id, 'name', v.name,
                          'email', v.email, 'employee_code', v.employee_code
                      ) ORDER BY v.name) FILTER (WHERE v.user_id IS NOT NULL),
                      '[]'::json
                  ) AS "members!: sqlx::types::Json<Vec<TeamMemberRef>>",
                  COALESCE(
                      (SELECT json_agg(jsonb_build_object(
                                  'user_id', m.id, 'name', m.name,
                                  'email', m.email, 'employee_code', m.employee_code)
                              ORDER BY m.name)::json
                       FROM team_pms tp
                       JOIN users m ON m.id = tp.pm_user_id
                       WHERE tp.team_id = t.id),
                      '[]'::json
                  ) AS "pms!: sqlx::types::Json<Vec<TeamMemberRef>>"
           FROM teams t
           LEFT JOIN visible v ON v.team_id = t.id
           GROUP BY t.id, t.name, t.description, t.created_at
           HAVING $1::uuid IS NULL OR COUNT(v.user_id) > 0
           ORDER BY t.name"#,
        manager_id
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| TeamDetail {
            team_id: r.id,
            name: r.name,
            description: r.description,
            member_count: r.member_count,
            members: r.members.0,
            pms: r.pms.0,
            created_at: r.created_at,
        })
        .collect())
}

// ─────────────────────────── team ↔ PM (migration 0045) ───────────────────────────

/// Is `user_id` a project manager of `team_id`?
///
/// THE authorization primitive for every team-scoped read. Deliberately its own function rather
/// than a clause repeated at each call site: "may this caller ask about this team" is one question
/// and should have one answer, so a new endpoint cannot accidentally invent a looser version of it.
pub async fn is_team_pm(pool: &PgPool, team_id: Uuid, user_id: Uuid) -> Result<bool, AppError> {
    let hit = sqlx::query_scalar!(
        "SELECT EXISTS (SELECT 1 FROM team_pms WHERE team_id = $1 AND pm_user_id = $2)",
        team_id,
        user_id
    )
    .fetch_one(pool)
    .await?;
    Ok(hit.unwrap_or(false))
}

/// The PMs assigned to one team, by name.
pub async fn pms_of(pool: &PgPool, team_id: Uuid) -> Result<Vec<TeamMemberRef>, AppError> {
    let rows = sqlx::query!(
        "SELECT u.id, u.name, u.email, u.employee_code
         FROM team_pms tp JOIN users u ON u.id = tp.pm_user_id
         WHERE tp.team_id = $1 ORDER BY u.name",
        team_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| TeamMemberRef {
            user_id: r.id,
            name: r.name,
            email: r.email,
            employee_code: r.employee_code,
        })
        .collect())
}

/// Assign a PM to a team. Idempotent, so a retry after a timeout is safe and the
/// caller needs no bookkeeping of its own.
pub async fn add_pm(
    pool: &PgPool,
    team_id: Uuid,
    pm_user_id: Uuid,
    added_by: Uuid,
) -> Result<(), AppError> {
    sqlx::query!(
        "INSERT INTO team_pms (team_id, pm_user_id, added_by) VALUES ($1, $2, $3)
         ON CONFLICT (team_id, pm_user_id) DO NOTHING",
        team_id,
        pm_user_id,
        added_by
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Unassign a PM. Also idempotent — removing someone who is already not a PM is
/// the state the caller wanted, not an error.
pub async fn remove_pm(pool: &PgPool, team_id: Uuid, pm_user_id: Uuid) -> Result<(), AppError> {
    sqlx::query!(
        "DELETE FROM team_pms WHERE team_id = $1 AND pm_user_id = $2",
        team_id,
        pm_user_id
    )
    .execute(pool)
    .await?;
    Ok(())
}
