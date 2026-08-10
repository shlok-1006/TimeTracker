//! Team-scoped attendance and live status, and the authorization behind them.
//!
//! The HRMS asked for these because the two halves of a PM's dashboard described different groups
//! of people: performance was team-scoped on their side, attendance manager-scoped on ours. They
//! also suggested we skip the server-side check and trust their proxy, which already knows who
//! owns which team. We declined — a PM holds a real access token and can call this API directly,
//! so a check only one client performs is not a check.
//!
//! These tests are mostly about that refusal holding: a PM assigned to a team gets it, the same PM
//! gets 404 for a team they are not assigned to, and an employee cannot reach any of it.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;
use uuid::Uuid;

use server::jwt::JwtKeys;
use server::linear_service::LinearService;
use server::role::UserRole;
use server::storage::{S3Config, StorageClient};
use server::AppState;

const SECRET: &str = "team-scope-test-secret";

/// The router over a REAL pool where `DATABASE_URL` is set. The other suites here point at a
/// credential-less localhost URL, which is fine when a test only asserts 401/403 — those are
/// decided before any query runs — but this file asserts a 200, so the pool has to work.
fn app() -> axum::Router {
    let url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://localhost/timetracker".into());
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy(&url)
        .expect("lazy pool");
    server::build_router(AppState::new(
        pool,
        JwtKeys::new(SECRET, 900),
        StorageClient::new(S3Config::insecure_local()),
        LinearService::from_env(),
        server::claude_provider::ClaudeProvider::from_env(),
        2_592_000,
    ))
}

fn token_for(id: Uuid, role: UserRole) -> String {
    JwtKeys::new(SECRET, 900)
        .issue(id, role, None, None)
        .unwrap()
}

async fn status(path: &str, who: Option<(Uuid, UserRole)>) -> StatusCode {
    let mut b = Request::builder().uri(path);
    if let Some((id, role)) = who {
        b = b.header("Authorization", format!("Bearer {}", token_for(id, role)));
    }
    app()
        .oneshot(b.body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

// ---- RBAC, no DB needed ----

#[tokio::test]
async fn the_team_reads_need_a_session() {
    let team = Uuid::new_v4();
    for path in [
        format!("/admin/teams/{team}/attendance?from=2020-01-01&to=2020-01-31"),
        format!("/admin/teams/{team}/live"),
        format!("/admin/teams/{team}/pms"),
    ] {
        assert_eq!(
            status(&path, None).await,
            StatusCode::UNAUTHORIZED,
            "{path}"
        );
    }
}

#[tokio::test]
async fn employees_cannot_reach_team_data() {
    let team = Uuid::new_v4();
    let emp = Uuid::new_v4();
    for path in [
        format!("/admin/teams/{team}/attendance?from=2020-01-01&to=2020-01-31"),
        format!("/admin/teams/{team}/live"),
        format!("/admin/teams/{team}/pms"),
    ] {
        assert_eq!(
            status(&path, Some((emp, UserRole::Employee))).await,
            StatusCode::FORBIDDEN,
            "{path}"
        );
    }
}

// ---- DB-backed: the scope itself ----

async fn real_pool() -> Option<sqlx::PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .ok()
}

async fn seed_user(pool: &sqlx::PgPool, role: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, name, email, password_hash, role)
         VALUES ($1, 'Scope Test', $2, 'x', $3::user_role)",
    )
    .bind(id)
    .bind(format!("scope-{id}@test.local"))
    .bind(role)
    .execute(pool)
    .await
    .expect("seed user");
    id
}

async fn seed_team(pool: &sqlx::PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO teams (id, name, description) VALUES ($1, $2, '')")
        .bind(id)
        .bind(format!("Scope Test {id}"))
        .execute(pool)
        .await
        .expect("seed team");
    id
}

#[tokio::test]
async fn a_pm_sees_their_own_team_and_only_their_own() {
    let Some(pool) = real_pool().await else {
        eprintln!("DATABASE_URL unset — skipping team-scope test");
        return;
    };

    let pm = seed_user(&pool, "project_manager").await;
    let mine = seed_team(&pool).await;
    let theirs = seed_team(&pool).await;
    sqlx::query("INSERT INTO team_pms (team_id, pm_user_id) VALUES ($1, $2)")
        .bind(mine)
        .bind(pm)
        .execute(&pool)
        .await
        .expect("assign pm");

    for kind in ["live", "pms"] {
        assert_eq!(
            status(
                &format!("/admin/teams/{mine}/{kind}"),
                Some((pm, UserRole::ProjectManager))
            )
            .await,
            StatusCode::OK,
            "a PM must reach their own team's {kind}"
        );
        assert_eq!(
            status(
                &format!("/admin/teams/{theirs}/{kind}"),
                Some((pm, UserRole::ProjectManager))
            )
            .await,
            StatusCode::NOT_FOUND,
            "and must NOT reach a team they are not assigned to — this is the check the HRMS \
             proposed we skip, so it is the one that must not rot"
        );
    }

    sqlx::query("DELETE FROM teams WHERE id = ANY($1)")
        .bind(vec![mine, theirs])
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(pm)
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
async fn attendance_lists_the_team_not_the_managed_subtree() {
    let Some(pool) = real_pool().await else {
        eprintln!("DATABASE_URL unset — skipping team attendance test");
        return;
    };

    let pm = seed_user(&pool, "project_manager").await;
    let on_team = seed_user(&pool, "employee").await;
    let managed_only = seed_user(&pool, "employee").await;
    let team = seed_team(&pool).await;

    sqlx::query("INSERT INTO team_pms (team_id, pm_user_id) VALUES ($1, $2)")
        .bind(team)
        .bind(pm)
        .execute(&pool)
        .await
        .expect("assign pm");
    sqlx::query("INSERT INTO user_teams (user_id, team_id) VALUES ($1, $2)")
        .bind(on_team)
        .bind(team)
        .execute(&pool)
        .await
        .expect("add member");
    // Managed by the PM but NOT on the team — the difference the whole change is about.
    sqlx::query("INSERT INTO user_managers (user_id, manager_id) VALUES ($1, $2)")
        .bind(managed_only)
        .bind(pm)
        .execute(&pool)
        .await
        .expect("assign manager");

    let rows = server::db::attendance::report_for_team(
        &pool,
        chrono::NaiveDate::from_ymd_opt(2020, 1, 1).unwrap(),
        chrono::NaiveDate::from_ymd_opt(2020, 1, 31).unwrap(),
        team,
    )
    .await
    .expect("team attendance");

    let ids: Vec<Uuid> = rows.iter().map(|r| r.user_id).collect();
    assert!(ids.contains(&on_team), "a team member must be listed");
    assert!(
        !ids.contains(&managed_only),
        "someone the PM manages but who is NOT on the team must not appear — membership decides \
         the list, management decides nothing here"
    );
    assert!(
        !ids.contains(&pm),
        "the PM is not staff of their own team unless someone also put them on it"
    );
    // Present in the range with no attendance rows reads as zeros, not as a missing person.
    let row = rows.iter().find(|r| r.user_id == on_team).unwrap();
    assert_eq!(row.present + row.absent + row.leave, 0);
    assert_eq!(row.worked_seconds, 0);

    sqlx::query("DELETE FROM teams WHERE id = $1")
        .bind(team)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM users WHERE id = ANY($1)")
        .bind(vec![pm, on_team, managed_only])
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
async fn assigning_a_pm_is_idempotent_and_reversible() {
    let Some(pool) = real_pool().await else {
        eprintln!("DATABASE_URL unset — skipping pm assignment test");
        return;
    };
    let pm = seed_user(&pool, "project_manager").await;
    let hr = seed_user(&pool, "hr").await;
    let team = seed_team(&pool).await;

    // Twice: a retry after a timeout must not be an error, nor a second row.
    for _ in 0..2 {
        server::db::teams::add_pm(&pool, team, pm, hr)
            .await
            .expect("add pm");
    }
    assert_eq!(
        server::db::teams::pms_of(&pool, team).await.unwrap().len(),
        1
    );
    assert!(server::db::teams::is_team_pm(&pool, team, pm)
        .await
        .unwrap());

    // And removing twice is the state the caller asked for, not a failure.
    for _ in 0..2 {
        server::db::teams::remove_pm(&pool, team, pm)
            .await
            .expect("remove pm");
    }
    assert!(server::db::teams::pms_of(&pool, team)
        .await
        .unwrap()
        .is_empty());
    assert!(!server::db::teams::is_team_pm(&pool, team, pm)
        .await
        .unwrap());

    sqlx::query("DELETE FROM teams WHERE id = $1")
        .bind(team)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM users WHERE id = ANY($1)")
        .bind(vec![pm, hr])
        .execute(&pool)
        .await
        .ok();
}
