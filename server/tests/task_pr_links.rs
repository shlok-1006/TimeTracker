//! PR links on tasks + the per-team read the HRMS performance engine pulls.
//!
//! The engine's contract is: "give me the PR-bearing tasks for this team, with the owner's email".
//! So the tests pin exactly that — `list_for_team(has_pr=true)` returns only tasks that carry a PR,
//! scoped to the team's members, each with the email — plus that PR links round-trip through
//! create/update, and that the endpoint is staff-only.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;
use uuid::Uuid;

use server::db::manual_tasks;
use server::jwt::JwtKeys;
use server::linear_service::LinearService;
use server::role::UserRole;
use server::storage::{S3Config, StorageClient};
use server::AppState;

const SECRET: &str = "task-pr-test-secret";

async fn real_pool() -> Option<sqlx::PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .ok()
}

async fn seed_user(pool: &sqlx::PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, name, email, password_hash, role) VALUES ($1,'PR Test',$2,'x','employee')")
        .bind(id)
        .bind(format!("pr-{id}@test.local"))
        .execute(pool)
        .await
        .expect("seed user");
    id
}

async fn seed_team(pool: &sqlx::PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO teams (id, name, description) VALUES ($1,$2,'')")
        .bind(id)
        .bind(format!("PR Team {id}"))
        .execute(pool)
        .await
        .expect("seed team");
    id
}

#[tokio::test]
async fn pr_links_round_trip_and_team_read_filters_to_pr_bearing() {
    let Some(pool) = real_pool().await else {
        eprintln!("DATABASE_URL unset — skipping task pr-links test");
        return;
    };

    let team = seed_team(&pool).await;
    let member = seed_user(&pool).await;
    let outsider = seed_user(&pool).await;
    sqlx::query("INSERT INTO user_teams (user_id, team_id) VALUES ($1,$2)")
        .bind(member)
        .bind(team)
        .execute(&pool)
        .await
        .expect("add member");

    // A task WITH two PRs, and one without — both owned by the team member.
    let prs = vec![
        "https://github.com/ruh-ai/time-tracker/pull/42".to_string(),
        "https://github.com/ruh-ai/time-tracker/pull/43".to_string(),
    ];
    let with_pr = manual_tasks::create(&pool, member, member, "Ship feature", "", 7, None, &prs)
        .await
        .expect("create with pr");
    assert_eq!(with_pr.pr_links, prs, "pr_links round-trip through create");
    let _no_pr = manual_tasks::create(&pool, member, member, "Plain task", "", 5, None, &[])
        .await
        .expect("create without pr");
    // A PR task owned by someone NOT on the team — must never appear in the team read.
    let _outsider_task = manual_tasks::create(
        &pool,
        outsider,
        outsider,
        "Outsider PR",
        "",
        5,
        None,
        &["https://github.com/x/y/pull/1".to_string()],
    )
    .await
    .expect("create outsider");

    // has_pr = true → only the PR-bearing task of the team member, with their email.
    let engine = manual_tasks::list_for_team(&pool, team, true)
        .await
        .unwrap();
    assert_eq!(engine.len(), 1, "only the PR-bearing team task");
    let row = &engine[0];
    assert_eq!(row.task_id, with_pr.id);
    assert_eq!(row.user_id, member);
    assert!(row.user_email.contains(&member.to_string()[..8]) || row.user_email.contains('@'));
    assert_eq!(row.pr_links, prs);
    assert_eq!(row.weight, 7);

    // has_pr = false → both of the member's tasks, still never the outsider's.
    let all = manual_tasks::list_for_team(&pool, team, false)
        .await
        .unwrap();
    assert_eq!(all.len(), 2, "both team tasks");
    assert!(all.iter().all(|t| t.user_id == member));

    // Update replaces the PR set.
    let one = vec!["https://github.com/ruh-ai/time-tracker/pull/99".to_string()];
    manual_tasks::update(&pool, with_pr.id, None, None, None, None, Some(&one))
        .await
        .expect("update prs");
    let after = manual_tasks::get(&pool, with_pr.id).await.unwrap().unwrap();
    assert_eq!(after.pr_links, one, "pr_links replaced");
    assert_eq!(after.weight, 7, "COALESCE left weight alone");

    sqlx::query("DELETE FROM teams WHERE id = $1")
        .bind(team)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM users WHERE id = ANY($1)")
        .bind(vec![member, outsider])
        .execute(&pool)
        .await
        .ok();
}

// ---- RBAC over HTTP ----

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

async fn status(who: Option<(Uuid, UserRole)>) -> StatusCode {
    let team = Uuid::new_v4();
    let path = format!("/admin/teams/{team}/tasks?has_pr=1");
    let mut b = Request::builder().uri(path);
    if let Some((id, role)) = who {
        let token = JwtKeys::new(SECRET, 900)
            .issue(id, role, None, None)
            .unwrap();
        b = b.header("Authorization", format!("Bearer {token}"));
    }
    app()
        .oneshot(b.body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn team_tasks_is_staff_only() {
    assert_eq!(status(None).await, StatusCode::UNAUTHORIZED);
    assert_eq!(
        status(Some((Uuid::new_v4(), UserRole::Employee))).await,
        StatusCode::FORBIDDEN,
        "an employee cannot read a team's task feed"
    );
}
