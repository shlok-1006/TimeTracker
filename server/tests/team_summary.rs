//! Team summary tests (Feature 4 Phase 4): live metric aggregation over
//! intervals.team_id + HR/PM-only gating. Skips if DATABASE_URL is unset.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{Duration, TimeZone, Utc};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use server::db::intervals::{insert_batch, IntervalDto};
use server::db::{teams, users};
use server::jwt::JwtKeys;
use server::linear_service::LinearService;
use server::role::UserRole;
use server::storage::{GcsConfig, StorageClient};
use server::AppState;

const SECRET: &str = "team-summary-secret";

async fn pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    PgPoolOptions::new().max_connections(2).connect(&url).await.ok()
}

fn dto(offset_min: i64, dur_secs: i64, kind: &str, team: Uuid) -> IntervalDto {
    let start = Utc.with_ymd_and_hms(2099, 2, 1, 9, 0, 0).unwrap() + Duration::minutes(offset_min);
    IntervalDto {
        id: Uuid::new_v4(),
        start_utc: start,
        end_utc: start + Duration::seconds(dur_secs),
        kind: kind.into(),
        team_id: Some(team),
    }
}

#[tokio::test]
async fn team_summary_metrics() {
    let Some(pool) = pool().await else {
        eprintln!("skipping team_summary test: DATABASE_URL not set");
        return;
    };
    let tag = Uuid::new_v4();
    let team = teams::create(&pool, &format!("Sum-{tag}"), "summary team").await.unwrap();
    let e1 = users::create(&pool, "M1", &format!("m1-{tag}@t.local"), "h", UserRole::Employee, None).await.unwrap();
    let e2 = users::create(&pool, "M2", &format!("m2-{tag}@t.local"), "h", UserRole::Employee, None).await.unwrap();
    teams::add_member(&pool, e1.id, team.id).await.unwrap();
    teams::add_member(&pool, e2.id, team.id).await.unwrap();

    // e1: active 3600 + meeting 1800 → worked 5400 ; e2: active 1800 + idle 600 → worked 1800.
    insert_batch(&pool, e1.id, &[dto(0, 3600, "active", team.id), dto(60, 1800, "meeting", team.id)]).await.unwrap();
    insert_batch(&pool, e2.id, &[dto(120, 1800, "active", team.id), dto(180, 600, "idle", team.id)]).await.unwrap();

    let b = teams::status_breakdown(&pool, team.id).await.unwrap();
    assert_eq!(b.total, 7200, "total worked = active+meeting");
    assert_eq!(b.active, 5400);
    assert_eq!(b.meeting, 1800);
    assert_eq!(b.idle, 600);
    assert_eq!(b.break_, 0);

    let members = teams::member_totals(&pool, team.id).await.unwrap();
    assert_eq!(members.len(), 2);
    assert_eq!(members[0].user_id, e1.id, "highest worked first");
    assert_eq!(members[0].worked_seconds, 5400);
    assert_eq!(members[1].worked_seconds, 1800);
    let active_users = members.iter().filter(|m| m.worked_seconds > 0).count();
    assert_eq!(active_users, 2);

    // teams index includes member counts.
    let listed = teams::list_with_counts(&pool).await.unwrap();
    let row = listed.iter().find(|t| t.id == team.id).unwrap();
    assert_eq!(row.member_count, 2);

    // Cleanup: deleting users cascades their intervals; then the team.
    users::delete(&pool, e1.id).await.unwrap();
    users::delete(&pool, e2.id).await.unwrap();
    teams::delete(&pool, team.id).await.unwrap();
}

// ---- gating ----

fn lazy_app() -> axum::Router {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://localhost/timetracker")
        .expect("lazy pool");
    server::build_router(AppState::new(
        pool,
        JwtKeys::new(SECRET, 900),
        StorageClient::new(GcsConfig::from_env()),
        LinearService::from_env(),
        server::gemini_provider::GeminiProvider::from_env(),
        2_592_000,
    ))
}

async fn status(path: &str, role: Option<UserRole>) -> StatusCode {
    let mut b = Request::builder().uri(path);
    if let Some(r) = role {
        let t = JwtKeys::new(SECRET, 900).issue(Uuid::new_v4(), r, None).unwrap();
        b = b.header("authorization", format!("Bearer {t}"));
    }
    lazy_app().oneshot(b.body(Body::empty()).unwrap()).await.unwrap().status()
}

#[tokio::test]
async fn summary_requires_admin() {
    assert_eq!(status("/admin/teams", None).await, StatusCode::UNAUTHORIZED);
    assert_eq!(status("/admin/teams", Some(UserRole::Employee)).await, StatusCode::FORBIDDEN);
    let id = Uuid::new_v4();
    assert_eq!(
        status(&format!("/admin/teams/{id}/summary"), Some(UserRole::Employee)).await,
        StatusCode::FORBIDDEN
    );
}
