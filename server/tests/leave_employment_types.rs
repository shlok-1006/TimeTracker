//! Who may take a leave type, by employment type (migration 0055): Earned Leave is for employees only.
//!
//! Under test: an intern or contractor gets 0 days of an employees-only type — even when its defaults are not
//! 0 and even with an HR allocation — with the reason said, can't request it, and it drops out of their
//! "leaves left"; a project manager whose employment type says intern still counts as an employee; HR can't
//! allocate it to an intern (0 days is fine); and the leave-type API sets, keeps and refuses the list.
//! Needs a database: skipped when DATABASE_URL is unset. Uses year-2035 dates, fresh users and types, and
//! cleans up after.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use chrono::NaiveDate;
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use server::db::{leave, users};
use server::employment_type::EmploymentType;
use server::jwt::JwtKeys;
use server::leave_service;
use server::linear_service::LinearService;
use server::role::UserRole;
use server::storage::{S3Config, StorageClient};
use server::AppState;

const SECRET: &str = "leave-employment-types-test-secret";
const YEAR: i32 = 2035;

/// The "leaves left" assertion sums every paid type: run the tests here one at a time.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn real_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .ok()
}

fn app_with(pool: PgPool) -> Router {
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

async fn send(
    app: Router,
    method: &str,
    path: &str,
    tok: &str,
    body: Value,
) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {tok}"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn person(pool: &PgPool, tag: &str, who: &str, role: UserRole, et: EmploymentType) -> Uuid {
    let u = users::create(pool, who, &format!("{who}-{tag}@t.local"), "h", role, None)
        .await
        .unwrap();
    assert!(users::set_employment_type(pool, u.id, et).await.unwrap());
    u.id
}

async fn balance(pool: &PgPool, user: Uuid, lt: Uuid) -> leave::Balance {
    leave::balances(pool, user, YEAR)
        .await
        .unwrap()
        .into_iter()
        .find(|b| b.leave_type_id == lt)
        .expect("the test type is in the balance")
}

async fn cleanup(pool: &PgPool, people: &[Uuid], types: &[Uuid]) {
    for id in people {
        for sql in [
            "DELETE FROM leave_requests WHERE user_id = $1",
            "DELETE FROM leave_allocations WHERE user_id = $1",
            "DELETE FROM users WHERE id = $1",
        ] {
            sqlx::query(sql).bind(id).execute(pool).await.unwrap();
        }
    }
    for id in types {
        sqlx::query("DELETE FROM leave_types WHERE id = $1")
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn employees_only_leave_is_never_an_interns_or_contractors() {
    let _serial = SERIAL.lock().await;
    let Some(pool) = real_pool().await else {
        eprintln!("skipping leave employment-type test: DATABASE_URL not set");
        return;
    };
    let tag = Uuid::new_v4().simple().to_string();
    let employee = person(
        &pool,
        &tag,
        "el-emp",
        UserRole::Employee,
        EmploymentType::Employee,
    )
    .await;
    let intern = person(
        &pool,
        &tag,
        "el-intern",
        UserRole::Employee,
        EmploymentType::Intern,
    )
    .await;
    let contractor = person(
        &pool,
        &tag,
        "el-contr",
        UserRole::Employee,
        EmploymentType::Contractor,
    )
    .await;
    // A PM counts as an employee for leave, whatever their employment type says.
    let pm = person(
        &pool,
        &tag,
        "el-pm",
        UserRole::ProjectManager,
        EmploymentType::Intern,
    )
    .await;

    // Defaults of 12 for every category: without the rule an intern would get 12.
    let earned = leave::create_type_for(
        &pool,
        &format!("earned-{tag}"),
        true,
        12.0,
        12.0,
        12.0,
        &leave::TypeRules::default(),
        &[EmploymentType::Employee],
    )
    .await
    .unwrap();
    assert_eq!(earned.employment_types, vec![EmploymentType::Employee]);
    // ...and an HR allocation written straight to the table (the route refuses it, below) changes nothing.
    leave::upsert_allocation(&pool, intern, earned.id, YEAR, 5.0)
        .await
        .unwrap();

    let b = balance(&pool, employee, earned.id).await;
    assert_eq!((b.allotted_days, b.eligible), (12.0, true));
    let b = balance(&pool, pm, earned.id).await;
    assert_eq!(
        (b.allotted_days, b.eligible),
        (12.0, true),
        "a PM counts as an employee"
    );

    let b = balance(&pool, intern, earned.id).await;
    assert_eq!(
        (b.allotted_days, b.remaining_days, b.eligible, b.is_override),
        (0.0, 0.0, false, false),
        "0 for an intern, whatever was allocated"
    );
    assert_eq!(
        b.eligibility_note.as_deref(),
        Some("Not available to interns")
    );
    assert_eq!(b.eligible_from, None);
    let b = balance(&pool, contractor, earned.id).await;
    assert_eq!((b.allotted_days, b.eligible), (0.0, false));
    assert_eq!(
        b.eligibility_note.as_deref(),
        Some("Not available to contractors")
    );

    // 2035-03-05 is a Monday.
    let day = NaiveDate::from_ymd_opt(YEAR, 3, 5).unwrap();
    let refused =
        leave_service::submit_request(&pool, intern, earned.id, day, day, "", None, None).await;
    let msg = format!(
        "{:?}",
        refused.expect_err("an intern can't request Earned Leave")
    );
    assert!(msg.contains("Not available to interns"), "{msg}");
    leave_service::submit_request(&pool, employee, earned.id, day, day, "", None, None)
        .await
        .expect("an employee can");

    // "Leaves left" leaves the type out for the intern: opening it to everyone adds their 5 allocated days.
    let left = |m: &std::collections::HashMap<Uuid, f64>| m.get(&intern).copied().unwrap_or(0.0);
    let before = left(
        &leave::remaining_paid_by_user(&pool, YEAR, None)
            .await
            .unwrap(),
    );
    let opened = leave::update_type_for(
        &pool,
        earned.id,
        true,
        12.0,
        12.0,
        12.0,
        None,
        Some(&leave::all_employment_types()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(opened.employment_types.len(), 3);
    let after = left(
        &leave::remaining_paid_by_user(&pool, YEAR, None)
            .await
            .unwrap(),
    );
    assert_eq!(after - before, 5.0);
    assert_eq!(balance(&pool, intern, earned.id).await.allotted_days, 5.0);

    // Editing days without the list keeps it (an older HRMS never resets a type to everyone).
    leave::update_type_for(
        &pool,
        earned.id,
        true,
        12.0,
        12.0,
        12.0,
        None,
        Some(&[EmploymentType::Employee]),
    )
    .await
    .unwrap();
    let kept = leave::update_type(&pool, earned.id, true, 10.0, 10.0, 10.0, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(kept.employment_types, vec![EmploymentType::Employee]);

    cleanup(&pool, &[employee, intern, contractor, pm], &[earned.id]).await;
}

#[tokio::test]
async fn hr_routes_set_the_list_and_refuse_allocating_it_to_an_intern() {
    let _serial = SERIAL.lock().await;
    let Some(pool) = real_pool().await else {
        eprintln!("skipping leave employment-type API test: DATABASE_URL not set");
        return;
    };
    let tag = Uuid::new_v4().simple().to_string();
    let hr = person(&pool, &tag, "el-hr", UserRole::Hr, EmploymentType::Employee).await;
    let intern = person(
        &pool,
        &tag,
        "el-intern2",
        UserRole::Employee,
        EmploymentType::Intern,
    )
    .await;
    let employee = person(
        &pool,
        &tag,
        "el-emp2",
        UserRole::Employee,
        EmploymentType::Employee,
    )
    .await;
    let tok = token_for(hr, UserRole::Hr);

    // Created for employees only (and a repeat in the list is dropped).
    let (s, t) = send(
        app_with(pool.clone()),
        "POST",
        "/admin/leave/types",
        &tok,
        json!({ "name": format!("earned-api-{tag}"), "default_days": 12, "default_days_intern": 12,
                "employment_types": ["employee", "employee"] }),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{t}");
    assert_eq!(t["employment_types"], json!(["employee"]));
    let id = t["id"].as_str().unwrap().to_string();

    // Created without a list: everyone.
    let (s, open) = send(
        app_with(pool.clone()),
        "POST",
        "/admin/leave/types",
        &tok,
        json!({ "name": format!("casual-api-{tag}"), "default_days": 6 }),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{open}");
    assert_eq!(
        open["employment_types"],
        json!(["employee", "contractor", "intern"])
    );
    let open_id = open["id"].as_str().unwrap().to_string();

    // An empty list is refused; an omitted one is kept.
    let path = format!("/admin/leave/types/{id}");
    let (s, _) = send(
        app_with(pool.clone()),
        "PATCH",
        &path,
        &tok,
        json!({ "default_days": 12, "employment_types": [] }),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, t) = send(
        app_with(pool.clone()),
        "PATCH",
        &path,
        &tok,
        json!({ "default_days": 11 }),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{t}");
    assert_eq!(t["employment_types"], json!(["employee"]));

    // Allocating it to an intern is refused, with the reason; 0 days is fine; an employee is fine.
    let alloc = |user: Uuid, days: f64| json!({ "user_id": user, "leave_type_id": id, "year": YEAR, "allotted_days": days });
    let (s, e) = send(
        app_with(pool.clone()),
        "POST",
        "/admin/leave/allocations",
        &tok,
        alloc(intern, 4.0),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{e}");
    assert!(e.to_string().contains("isn't available to interns"), "{e}");
    let (s, _) = send(
        app_with(pool.clone()),
        "POST",
        "/admin/leave/allocations",
        &tok,
        alloc(intern, 0.0),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = send(
        app_with(pool.clone()),
        "POST",
        "/admin/leave/allocations",
        &tok,
        alloc(employee, 4.0),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    // Adding days by adjustment is refused the same way; taking them away is not.
    let adjust = |delta: f64| json!({ "user_id": intern, "leave_type_id": id, "year": YEAR, "delta": delta });
    let (s, _) = send(
        app_with(pool.clone()),
        "POST",
        "/admin/leave/allocations/adjust",
        &tok,
        adjust(2.0),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = send(
        app_with(pool.clone()),
        "POST",
        "/admin/leave/allocations/adjust",
        &tok,
        adjust(-1.0),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let types: Vec<Uuid> = [id.as_str(), open_id.as_str()]
        .iter()
        .map(|s| s.parse().unwrap())
        .collect();
    cleanup(&pool, &[intern, employee, hr], &types).await;
}
