//! Monthly report tests: RBAC gating on every route, plus a DB-backed
//! end-to-end check that the aggregation actually rolls a month up correctly
//! (worked seconds, attendance counts, and the "days above the threshold"
//! figure HR reads the report for).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::NaiveDate;
use tower::ServiceExt;
use uuid::Uuid;

use server::jwt::JwtKeys;
use server::linear_service::LinearService;
use server::role::UserRole;
use server::storage::{S3Config, StorageClient};
use server::AppState;

const SECRET: &str = "monthly-test-secret";

fn app() -> axum::Router {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://localhost/timetracker")
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

fn token(role: UserRole) -> String {
    JwtKeys::new(SECRET, 900)
        .issue(Uuid::new_v4(), role, None, None)
        .unwrap()
}

async fn req(method: &str, path: &str, role: Option<UserRole>) -> StatusCode {
    let mut b = Request::builder().method(method).uri(path);
    if let Some(r) = role {
        b = b.header("Authorization", format!("Bearer {}", token(r)));
    }
    if method == "POST" {
        b = b.header("content-type", "application/json");
    }
    let body = if method == "POST" {
        Body::from("{}")
    } else {
        Body::empty()
    };
    app().oneshot(b.body(body).unwrap()).await.unwrap().status()
}

// ---- RBAC ----

#[tokio::test]
async fn own_monthly_report_requires_a_session() {
    assert_eq!(
        req("GET", "/me/reports/monthly", None).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn every_role_can_read_their_own_month() {
    // The employee-visible half of the feature: each role reads its own summary.
    for role in [UserRole::Employee, UserRole::ProjectManager, UserRole::Hr] {
        let s = req("GET", "/me/reports/monthly", Some(role)).await;
        assert_ne!(
            s,
            StatusCode::FORBIDDEN,
            "{role:?} must reach its own report"
        );
        assert_ne!(s, StatusCode::UNAUTHORIZED, "{role:?} has a valid token");
    }
}

#[tokio::test]
async fn employees_cannot_read_the_roster_or_generate() {
    // Generation and cross-employee views are HR/PM only.
    assert_eq!(
        req("GET", "/admin/reports/monthly", Some(UserRole::Employee)).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        req("POST", "/admin/reports/monthly", Some(UserRole::Employee)).await,
        StatusCode::FORBIDDEN
    );
    let target = Uuid::new_v4();
    assert_eq!(
        req(
            "POST",
            &format!("/admin/users/{target}/reports/monthly"),
            Some(UserRole::Employee)
        )
        .await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn hr_and_pm_may_reach_the_roster() {
    for role in [UserRole::Hr, UserRole::ProjectManager] {
        let s = req("GET", "/admin/reports/monthly", Some(role)).await;
        assert_ne!(s, StatusCode::FORBIDDEN, "{role:?} may read the roster");
        assert_ne!(s, StatusCode::UNAUTHORIZED);
    }
}

// ---- DB-backed: the aggregation itself ----

async fn real_pool() -> Option<sqlx::PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .ok()
}

#[tokio::test]
async fn rolls_up_a_month_from_attendance_and_daily_reports() {
    let Some(pool) = real_pool().await else {
        eprintln!("DATABASE_URL unset — skipping DB-backed monthly rollup test");
        return;
    };

    // A throwaway user in a long-past month so live data can never collide.
    let uid = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO users (id, name, email, password_hash, role)
         VALUES ($1, 'Monthly Test', $2, 'x', 'employee')",
        uid,
        format!("monthly-{uid}@test.local")
    )
    .execute(&pool)
    .await
    .expect("seed user");

    let month = NaiveDate::from_ymd_opt(2020, 4, 1).unwrap();
    // 3 present days (2h each) + 1 absent + 1 leave.
    for (d, status, secs) in [
        (1, "present", 7200),
        (2, "present", 7200),
        (3, "present", 7200),
        (6, "absent", 0),
        (7, "leave", 0),
    ] {
        sqlx::query!(
            "INSERT INTO attendance_days (user_id, day, status, worked_seconds, idle_seconds, note)
             VALUES ($1, $2, $3, $4, 0, '')",
            uid,
            NaiveDate::from_ymd_opt(2020, 4, d).unwrap(),
            status,
            secs as i32
        )
        .execute(&pool)
        .await
        .expect("seed attendance");
    }

    // Daily analysis: two days above 50%, one below.
    for (d, score) in [(1, 80.0_f64), (2, 55.0), (3, 20.0)] {
        let job = Uuid::new_v4();
        sqlx::query!(
            "INSERT INTO analysis_jobs (id, user_id, day) VALUES ($1, $2, $3)",
            job,
            uid,
            NaiveDate::from_ymd_opt(2020, 4, d).unwrap()
        )
        .execute(&pool)
        .await
        .expect("seed job");
        sqlx::query!(
            "INSERT INTO analysis_reports
               (user_id, day, job_id, total_analyzed, aligned_count, partially_count,
                not_aligned_count, inconclusive_count, alignment_score, summary_text, model)
             VALUES ($1,$2,$3,4,2,1,1,0,$4,'test','test-model')",
            uid,
            NaiveDate::from_ymd_opt(2020, 4, d).unwrap(),
            job,
            score
        )
        .execute(&pool)
        .await
        .expect("seed report");
    }

    let report = server::monthly_report_service::build(&pool, uid, month, None)
        .await
        .expect("build monthly report");

    assert_eq!(report.month, month);
    assert_eq!(report.worked_seconds, 21_600, "3 × 2h");
    assert_eq!(report.days_present, 3);
    assert_eq!(report.days_absent, 1);
    assert_eq!(report.days_leave, 1);
    assert_eq!(report.days_analyzed, 3);
    assert_eq!(
        report.days_above_threshold, 2,
        "80 and 55 clear the 50% threshold; 20 does not"
    );
    assert_eq!(report.screenshots_analyzed, 12, "3 days × 4 shots");
    let avg = report.avg_alignment_score.expect("average present");
    assert!(
        (avg - (80.0 + 55.0 + 20.0) / 3.0).abs() < 1e-9,
        "avg was {avg}"
    );
    assert_eq!(report.days.len(), 5, "one entry per attendance day");
    assert_eq!(report.generated_by, None, "scheduler-generated");

    // Regenerating must upsert (no duplicate row) and keep the same figures.
    let again = server::monthly_report_service::build(&pool, uid, month, None)
        .await
        .expect("regenerate");
    assert_eq!(again.id, report.id, "same row refreshed, not duplicated");
    assert_eq!(again.worked_seconds, report.worked_seconds);

    // Cleanup (cascades to attendance/reports/jobs via the user FK).
    sqlx::query!("DELETE FROM users WHERE id = $1", uid)
        .execute(&pool)
        .await
        .ok();
}
