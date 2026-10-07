//! Managers by assignment: an `employee` assigned as someone's manager in `user_managers` (a team
//! lead who isn't a project manager) sees THOSE people's work as a project manager does — hours,
//! timeline, attendance, activity, screenshots with AI verdicts, the daily AI report and analysis,
//! monthly reports — and approves their leave. Actions (running analysis, tasks, grace) and anyone
//! else stay closed. Their own role, and so their own tracking, is unchanged.
//!
//! What these tests hold the server to:
//!   · HR can assign any active user as a manager (not only project managers).
//!   · The lead reaches all of the above for their own people, gets 403 for anyone else, and 403
//!     for the PM/HR actions and HR-only surfaces.
//!   · Screenshots carry the AI verdict for the lead too, and the lead's view is audit-logged.
//!   · The lead approves their own person's leave, and can't approve anyone else's.
//!   · Direct reports only — a lead's own manager does not see the lead's people.
//!   · Removing the assignment removes the access on the very next request.
//!   · An employee with nobody assigned has none of it.
//!
//! Hits a live DB via DATABASE_URL; skips if unset.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{NaiveDate, TimeZone, Utc};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use server::db::{analysis_results, screenshots, users};
use server::jwt::JwtKeys;
use server::linear_service::LinearService;
use server::role::UserRole;
use server::sampler;
use server::storage::{S3Config, StorageClient};
use server::vision_analyzer::AnalysisResult;
use server::AppState;

const SECRET: &str = "assigned-managers-test-secret";

async fn pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .ok()
}

fn app(pool: PgPool) -> axum::Router {
    server::build_router(AppState::new(
        pool,
        JwtKeys::new(SECRET, 900),
        StorageClient::new(S3Config::insecure_local()),
        LinearService::from_env(),
        server::claude_provider::ClaudeProvider::from_env(),
        2_592_000,
    ))
}

async fn call(
    pool: &PgPool,
    method: &str,
    path: &str,
    who: (Uuid, UserRole),
    body: Option<Value>,
) -> (StatusCode, Value) {
    let token = JwtKeys::new(SECRET, 900)
        .issue(who.0, who.1, None, None)
        .unwrap();
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header("Authorization", format!("Bearer {token}"));
    let body = match body {
        Some(v) => {
            b = b.header("Content-Type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let res = app(pool.clone())
        .oneshot(b.body(body).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 4 << 20)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

async fn mk(pool: &PgPool, role: UserRole, tag: &str) -> Uuid {
    users::create(
        pool,
        &format!("AM {tag}"),
        &format!("am-{tag}-{}@t.local", Uuid::new_v4()),
        "h",
        role,
        None,
    )
    .await
    .unwrap()
    .id
}

/// PM/HR actions and HR-only surfaces that stay closed to an employee who manages people.
fn closed_paths(target: Uuid) -> Vec<(&'static str, String)> {
    vec![
        ("GET", format!("/admin/users/{target}/tasks")),
        ("GET", format!("/admin/users/{target}/teams")),
        (
            "POST",
            format!("/admin/users/{target}/analyze?day=2099-04-01"),
        ),
        (
            "POST",
            format!("/admin/users/{target}/reports/monthly?month=2026-09-01"),
        ),
        (
            "GET",
            "/admin/attendance?from=2099-04-01&to=2099-04-30".to_string(),
        ),
        ("GET", format!("/admin/users/{target}/managers")),
        ("GET", format!("/admin/users/{target}/reports")),
    ]
}

#[tokio::test]
async fn an_employee_lead_sees_only_their_people_and_only_the_granted_reads() {
    let Some(pool) = pool().await else {
        eprintln!("skipping assigned_managers test: DATABASE_URL not set");
        return;
    };
    let hr = mk(&pool, UserRole::Hr, "hr").await;
    let pm = mk(&pool, UserRole::ProjectManager, "pm").await;
    let lead = mk(&pool, UserRole::Employee, "lead").await; // e.g. Nikita
    let mine = mk(&pool, UserRole::Employee, "intern-a").await; // assigned to the lead
    let other = mk(&pool, UserRole::Employee, "intern-b").await; // not assigned to the lead
    let boss = mk(&pool, UserRole::Employee, "lead-of-lead").await; // manages the lead
    let nobody = mk(&pool, UserRole::Employee, "plain").await; // manages no one

    // ---- HR may now assign an EMPLOYEE as a manager (it used to require project_manager) ----
    let (st, body) = call(
        &pool,
        "PUT",
        &format!("/admin/users/{mine}/managers"),
        (hr, UserRole::Hr),
        Some(json!({ "manager_ids": [lead] })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    users::set_managers(&pool, other, &[pm]).await.unwrap();
    users::set_managers(&pool, lead, &[boss]).await.unwrap();

    // ---- the lead's reads, for their own person ----
    let lead_who = (lead, UserRole::Employee);
    for path in [
        format!("/admin/users/{mine}/hours"),
        format!("/admin/users/{mine}/timeline?from=2099-04-01T00:00:00Z&to=2099-04-02T00:00:00Z"),
        format!("/admin/users/{mine}/attendance?from=2099-04-01&to=2099-04-07"),
        format!("/admin/users/{mine}/screenshots?day=2099-04-01"),
        format!("/admin/users/{mine}/activity?day=2099-04-01"),
        format!("/admin/users/{mine}/report?day=2099-04-01"),
        format!("/admin/users/{mine}/analysis?day=2099-04-01"),
        format!("/admin/users/{mine}/reports/monthly?month=2026-09-01"),
        "/admin/leave/requests".to_string(),
        "/admin/leave/calendar?from=2099-04-01&to=2099-04-30".to_string(),
    ] {
        let (st, body) = call(&pool, "GET", &path, lead_who, None).await;
        assert_eq!(st, StatusCode::OK, "{path}: {body}");
    }
    let (st, team) = call(&pool, "GET", "/admin/team", lead_who, None).await;
    assert_eq!(st, StatusCode::OK);
    let ids: Vec<String> = team
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["user"]["id"].as_str().unwrap().to_string())
        .collect();
    assert!(
        ids.contains(&mine.to_string()),
        "their person is on their roster"
    );
    assert!(
        !ids.contains(&other.to_string()),
        "someone else's person is not"
    );

    // ---- …but not for anyone else ----
    for path in [
        format!("/admin/users/{other}/hours"),
        format!("/admin/users/{other}/screenshots?day=2099-04-01"),
        format!("/admin/users/{other}/attendance?from=2099-04-01&to=2099-04-07"),
        format!("/admin/users/{other}/timeline?from=2099-04-01T00:00:00Z&to=2099-04-02T00:00:00Z"),
        format!("/admin/users/{other}/report?day=2099-04-01"),
        format!("/admin/users/{other}/analysis?day=2099-04-01"),
        format!("/admin/users/{other}/activity?day=2099-04-01"),
        format!("/admin/users/{other}/reports/monthly?month=2026-09-01"),
    ] {
        let (st, _) = call(&pool, "GET", &path, lead_who, None).await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{path}");
    }

    // ---- …and none of the PM-only surfaces, even for their own person ----
    for (method, path) in closed_paths(mine) {
        let (st, _) = call(&pool, method, &path, lead_who, None).await;
        assert_eq!(
            st,
            StatusCode::FORBIDDEN,
            "{method} {path} must stay closed to a lead"
        );
    }

    // ---- direct reports only: the lead's own manager doesn't see the lead's people ----
    let (st, _) = call(
        &pool,
        "GET",
        &format!("/admin/users/{lead}/hours"),
        (boss, UserRole::Employee),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "a lead's manager sees the lead");
    let (st, _) = call(
        &pool,
        "GET",
        &format!("/admin/users/{mine}/hours"),
        (boss, UserRole::Employee),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "but not the lead's people");

    // ---- an employee who manages nobody gets none of it ----
    for path in [
        "/admin/team".to_string(),
        format!("/admin/users/{mine}/hours"),
    ] {
        let (st, _) = call(&pool, "GET", &path, (nobody, UserRole::Employee), None).await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{path}");
    }

    // ---- /me tells a client who manages people (live from the DB) ----
    let (_, me) = call(&pool, "GET", "/me", lead_who, None).await;
    assert_eq!(me["manages"], json!(true));
    let (_, me) = call(&pool, "GET", "/me", (nobody, UserRole::Employee), None).await;
    assert_eq!(me["manages"], json!(false));

    // ---- removing the assignment removes the access on the next request ----
    users::set_managers(&pool, mine, &[]).await.unwrap();
    let (st, _) = call(
        &pool,
        "GET",
        &format!("/admin/users/{mine}/hours"),
        lead_who,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "access ends with the assignment");
    let (st, _) = call(&pool, "GET", "/admin/team", lead_who, None).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "no people left → no team view");

    for id in [hr, pm, lead, mine, other, boss, nobody] {
        users::delete(&pool, id).await.unwrap();
    }
}

#[tokio::test]
async fn screenshots_reach_a_lead_with_the_ai_verdict_and_are_audited() {
    let Some(pool) = pool().await else {
        eprintln!("skipping assigned_managers test: DATABASE_URL not set");
        return;
    };
    let pm = mk(&pool, UserRole::ProjectManager, "pm-shots").await;
    let lead = mk(&pool, UserRole::Employee, "lead-shots").await;
    let person = mk(&pool, UserRole::Employee, "person-shots").await;
    users::set_managers(&pool, person, &[pm, lead])
        .await
        .unwrap();

    let day = NaiveDate::from_ymd_opt(2099, 4, 1).unwrap();
    let job = sampler::create_daily_job(&pool, person, day).await.unwrap();
    let shot = screenshots::insert(
        &pool,
        person,
        &format!("{person}/a/work.jpg"),
        Utc.from_utc_datetime(&day.and_hms_opt(9, 0, 0).unwrap()),
        None,
        "working",
    )
    .await
    .unwrap();
    analysis_results::upsert(
        &pool,
        job.id,
        shot,
        &AnalysisResult {
            verdict: "aligned".into(),
            matched_ticket_id: None,
            confidence: 0.9,
            observed: "x".into(),
            rationale: "y".into(),
            inconclusive_reason: None,
            model: "m".into(),
        },
    )
    .await
    .unwrap();

    let path = format!("/admin/users/{person}/screenshots?day=2099-04-01");
    let (st, pm_view) = call(&pool, "GET", &path, (pm, UserRole::ProjectManager), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        pm_view[0]["verdict"],
        json!("aligned"),
        "a PM still gets the verdict"
    );

    let (st, lead_view) = call(&pool, "GET", &path, (lead, UserRole::Employee), None).await;
    assert_eq!(st, StatusCode::OK);
    let item = lead_view[0].as_object().expect("one screenshot");
    assert_eq!(
        item["verdict"],
        json!("aligned"),
        "a lead sees the AI verdict like a PM"
    );
    assert!(
        item.contains_key("presigned_url"),
        "the image itself is shown"
    );

    let audited: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_logs
          WHERE actor_id = $1 AND action = 'screenshot.view_day' AND entity_id = $2",
    )
    .bind(lead)
    .bind(person)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audited, 1, "the lead's screenshot view is audit-logged");

    for id in [pm, lead, person] {
        users::delete(&pool, id).await.unwrap();
    }
}

#[tokio::test]
async fn hr_assigns_a_leads_people_in_one_place() {
    let Some(pool) = pool().await else {
        eprintln!("skipping assigned_managers test: DATABASE_URL not set");
        return;
    };
    let hr = mk(&pool, UserRole::Hr, "hr-rep").await;
    let pm = mk(&pool, UserRole::ProjectManager, "pm-rep").await;
    let lead = mk(&pool, UserRole::Employee, "lead-rep").await;
    let a = mk(&pool, UserRole::Employee, "a-rep").await;
    let b = mk(&pool, UserRole::Employee, "b-rep").await;
    let gone = mk(&pool, UserRole::Employee, "gone-rep").await;
    users::deactivate(&pool, gone, hr).await.unwrap();
    users::set_managers(&pool, a, &[pm]).await.unwrap(); // a already has a PM
                                                         // The lead used to lead someone who has since been deactivated: saving the editor (which only
                                                         // lists active people) must not drop that link.
    let former = mk(&pool, UserRole::Employee, "former-rep").await;
    users::set_managers(&pool, former, &[lead]).await.unwrap();
    users::deactivate(&pool, former, hr).await.unwrap();

    let hr_who = (hr, UserRole::Hr);
    let reports = format!("/admin/users/{lead}/reports");

    let (st, body) = call(
        &pool,
        "PUT",
        &reports,
        hr_who,
        Some(json!({ "user_ids": [a, b] })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert_eq!(body.as_array().unwrap().len(), 2);
    // The lead was ADDED as a's manager; a's PM is untouched.
    let a_mgrs: Vec<Uuid> = users::managers_of(&pool, a)
        .await
        .unwrap()
        .into_iter()
        .map(|(id, _, _)| id)
        .collect();
    assert!(a_mgrs.contains(&pm) && a_mgrs.contains(&lead));

    // Replacing the set drops b only.
    let (st, body) = call(
        &pool,
        "PUT",
        &reports,
        hr_who,
        Some(json!({ "user_ids": [a] })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body.as_array().unwrap().len(), 1);
    assert!(!users::is_manager_of(&pool, lead, b).await.unwrap());
    assert!(
        users::is_manager_of(&pool, lead, former).await.unwrap(),
        "a deactivated person's link survives a save"
    );

    // Refusals: managing yourself, a deactivated person, and anyone who isn't HR.
    let (st, _) = call(
        &pool,
        "PUT",
        &reports,
        hr_who,
        Some(json!({ "user_ids": [lead] })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _) = call(
        &pool,
        "PUT",
        &reports,
        hr_who,
        Some(json!({ "user_ids": [gone] })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _) = call(
        &pool,
        "PUT",
        &format!("/admin/users/{b}/managers"),
        hr_who,
        Some(json!({ "manager_ids": [gone] })),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::BAD_REQUEST,
        "a deactivated user can't be a manager"
    );
    // No mutual management: `a` (whom the lead manages) can't become the lead's manager, from either side.
    let (st, body) = call(
        &pool,
        "PUT",
        &format!("/admin/users/{lead}/managers"),
        hr_who,
        Some(json!({ "manager_ids": [a] })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{body}");
    let (st, _) = call(
        &pool,
        "PUT",
        &format!("/admin/users/{a}/reports"),
        hr_who,
        Some(json!({ "user_ids": [lead] })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "nor via a's Manages list");
    assert!(!users::is_manager_of(&pool, a, lead).await.unwrap());

    for who in [(pm, UserRole::ProjectManager), (lead, UserRole::Employee)] {
        let (st, _) = call(&pool, "PUT", &reports, who, Some(json!({ "user_ids": [] }))).await;
        assert_eq!(st, StatusCode::FORBIDDEN);
    }

    // managers_with_roles tells an employee-manager apart from a PM (emails depend on it).
    let roles: Vec<UserRole> = users::managers_with_roles(&pool, a)
        .await
        .unwrap()
        .into_iter()
        .map(|(_, _, _, r)| r)
        .collect();
    assert!(roles.contains(&UserRole::Employee) && roles.contains(&UserRole::ProjectManager));

    for id in [hr, pm, lead, a, b, gone, former] {
        users::delete(&pool, id).await.unwrap();
    }
}

#[tokio::test]
async fn a_lead_approves_their_own_peoples_leave_and_no_one_elses() {
    let Some(pool) = pool().await else {
        eprintln!("skipping assigned_managers test: DATABASE_URL not set");
        return;
    };
    let pm = mk(&pool, UserRole::ProjectManager, "pm-leave").await;
    let lead = mk(&pool, UserRole::Employee, "lead-leave").await;
    let mine = mk(&pool, UserRole::Employee, "mine-leave").await;
    let other = mk(&pool, UserRole::Employee, "other-leave").await;
    users::set_managers(&pool, mine, &[lead]).await.unwrap();
    users::set_managers(&pool, other, &[pm]).await.unwrap();

    let lt: Uuid = sqlx::query_scalar(
        "INSERT INTO leave_types (name, paid, default_days, default_days_intern, default_days_contractor)
         VALUES ($1, true, 10, 10, 10) RETURNING id",
    )
    .bind(format!("AM leave {}", Uuid::new_v4()))
    .fetch_one(&pool)
    .await
    .unwrap();
    let day = NaiveDate::from_ymd_opt(2099, 4, 6).unwrap(); // a Monday
    let request = |owner: Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, Uuid>(
                "INSERT INTO leave_requests (user_id, leave_type_id, start_date, end_date, days, reason)
                 VALUES ($1, $2, $3, $3, 1, 'test') RETURNING id",
            )
            .bind(owner)
            .bind(lt)
            .bind(day)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let mine_req = request(mine).await;
    let other_req = request(other).await;

    let lead_who = (lead, UserRole::Employee);
    // The lead's queue holds their own person's request only.
    let (st, queue) = call(&pool, "GET", "/admin/leave/requests", lead_who, None).await;
    assert_eq!(st, StatusCode::OK);
    let ids: Vec<&str> = queue
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["id"].as_str())
        .collect();
    assert!(ids.contains(&mine_req.to_string().as_str()));
    assert!(!ids.contains(&other_req.to_string().as_str()));

    // Someone else's request: refused, and still pending.
    let (st, _) = call(
        &pool,
        "POST",
        &format!("/admin/leave/requests/{other_req}/approve"),
        lead_who,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    // Their own person's request: approved.
    let (st, body) = call(
        &pool,
        "POST",
        &format!("/admin/leave/requests/{mine_req}/approve"),
        lead_who,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    let status = |id: Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, String>("SELECT status FROM leave_requests WHERE id = $1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    assert_eq!(status(mine_req).await, "approved");
    assert_eq!(status(other_req).await, "pending");

    // The approval names the lead as approver, so the requests go before the users.
    sqlx::query("DELETE FROM leave_requests WHERE leave_type_id = $1")
        .bind(lt)
        .execute(&pool)
        .await
        .unwrap();
    for id in [pm, lead, mine, other] {
        users::delete(&pool, id).await.unwrap();
    }
    sqlx::query("DELETE FROM leave_types WHERE id = $1")
        .bind(lt)
        .execute(&pool)
        .await
        .unwrap();
}
