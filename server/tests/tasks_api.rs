//! Manual-task management API tests (Feature 5 Phase 2): dashboard-role gating
//! (no DB) plus live HTTP round-trips with audit + PM-scope verification (skip
//! if no DATABASE_URL).

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use server::db::users;
use server::jwt::JwtKeys;
use server::linear_service::LinearService;
use server::role::UserRole;
use server::storage::{S3Config, StorageClient};
use server::AppState;

const SECRET: &str = "tasks-api-test-secret";

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

fn lazy_app() -> Router {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://localhost/timetracker")
        .expect("lazy pool");
    app_with(pool)
}

fn token(role: UserRole) -> String {
    JwtKeys::new(SECRET, 900)
        .issue(Uuid::new_v4(), role, None, None)
        .unwrap()
}

async fn send(
    app: Router,
    method: &str,
    path: &str,
    tok: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(path);
    if let Some(t) = tok {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    let req = match body {
        Some(j) => b
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&j).unwrap()))
            .unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let v = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, v)
}

async fn real_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .ok()
}

#[tokio::test]
async fn task_management_rejects_employees_and_anon() {
    let uid = Uuid::new_v4();
    let path = format!("/admin/users/{uid}/tasks");
    // No token → 401.
    let (s, _) = send(
        lazy_app(),
        "POST",
        &path,
        None,
        Some(json!({ "title": "X" })),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    // Employees are desktop-only → 403 by role, before any DB lookup. (Project
    // managers now pass the role gate and are instead team-scoped, which needs a
    // real DB — covered by `pm_task_scope_over_http`.)
    let t = token(UserRole::Employee);
    let (s, _) = send(
        lazy_app(),
        "POST",
        &path,
        Some(&t),
        Some(json!({ "title": "X" })),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "employee must be forbidden");
    let (s2, _) = send(
        lazy_app(),
        "DELETE",
        &format!("/admin/tasks/{}", Uuid::new_v4()),
        Some(&t),
        None,
    )
    .await;
    assert_eq!(s2, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn task_crud_and_audit_over_http() {
    let Some(pool) = real_pool().await else {
        eprintln!("skipping tasks_api round-trip: DATABASE_URL not set");
        return;
    };

    // Log in as the seed HR so created_by + audit reference a real user.
    let (s, login) = send(
        app_with(pool.clone()),
        "POST",
        "/auth/login",
        None,
        Some(json!({ "email": "hr@timetracker.local", "password": "ChangeMe!HR1" })),
    )
    .await;
    if s != StatusCode::OK {
        eprintln!("skipping: seed HR login failed ({s})");
        return;
    }
    let hr = login["access_token"].as_str().unwrap().to_string();

    let tag = Uuid::new_v4();
    let emp = users::create(
        &pool,
        "Task Emp",
        &format!("taskemp-{tag}@t.local"),
        "h",
        UserRole::Employee,
        None,
    )
    .await
    .unwrap();

    // Create with a weight + due date.
    let (s, body) = send(
        app_with(pool.clone()),
        "POST",
        &format!("/admin/users/{}/tasks", emp.id),
        Some(&hr),
        Some(json!({
            "title": "Fix the gateway",
            "description": "retry logic",
            "weight": 8,
            "due_date": "2020-05-01"
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "create: {body}");
    let task_id = body["id"].as_str().unwrap().to_string();
    assert_eq!(body["status"], "open");
    assert_eq!(body["weight"], 8);
    assert_eq!(body["due_date"], "2020-05-01");

    // A weight outside 1–10 is rejected.
    let (s, _) = send(
        app_with(pool.clone()),
        "POST",
        &format!("/admin/users/{}/tasks", emp.id),
        Some(&hr),
        Some(json!({ "title": "bad", "weight": 11 })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "weight 11 must be rejected");

    // List.
    let (s, body) = send(
        app_with(pool.clone()),
        "GET",
        &format!("/admin/users/{}/tasks", emp.id),
        Some(&hr),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body.as_array().unwrap().len(), 1);

    // Update: mark done (title preserved).
    let (s, body) = send(
        app_with(pool.clone()),
        "PATCH",
        &format!("/admin/tasks/{task_id}"),
        Some(&hr),
        Some(json!({ "status": "done" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body["status"], "done");
    assert_eq!(body["title"], "Fix the gateway");

    // Invalid status → 400.
    let (s, _) = send(
        app_with(pool.clone()),
        "PATCH",
        &format!("/admin/tasks/{task_id}"),
        Some(&hr),
        Some(json!({ "status": "closed" })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Delete.
    let (s, _) = send(
        app_with(pool.clone()),
        "DELETE",
        &format!("/admin/tasks/{task_id}"),
        Some(&hr),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    // Audit: create + update + delete were all logged for this task.
    let tid = Uuid::parse_str(&task_id).unwrap();
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE entity_id = $1 AND action IN ('task.create','task.update','task.delete')",
    )
    .bind(tid)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audited, 3, "create/update/delete should each be audited");

    users::delete(&pool, emp.id).await.unwrap();
}

/// A project manager may assign tasks only to employees they manage.
#[tokio::test]
async fn pm_task_scope_over_http() {
    let Some(pool) = real_pool().await else {
        eprintln!("skipping pm_task_scope: DATABASE_URL not set");
        return;
    };
    let tag = Uuid::new_v4();
    let pm = users::create(
        &pool,
        "Scope PM",
        &format!("scopepm-{tag}@t.local"),
        "h",
        UserRole::ProjectManager,
        None,
    )
    .await
    .unwrap();
    let emp = users::create(
        &pool,
        "Scope Emp",
        &format!("scopeemp-{tag}@t.local"),
        "h",
        UserRole::Employee,
        None,
    )
    .await
    .unwrap();
    let pm_token = JwtKeys::new(SECRET, 900)
        .issue(pm.id, UserRole::ProjectManager, None, None)
        .unwrap();
    let path = format!("/admin/users/{}/tasks", emp.id);
    let body = json!({ "title": "scoped work", "weight": 3 });

    // Not a manager of emp yet → 403.
    let (s, _) = send(
        app_with(pool.clone()),
        "POST",
        &path,
        Some(&pm_token),
        Some(body.clone()),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "PM must not assign to an unmanaged employee"
    );

    // Assign PM as emp's manager → now allowed.
    users::add_manager(&pool, emp.id, pm.id).await.unwrap();
    let (s, created) = send(
        app_with(pool.clone()),
        "POST",
        &path,
        Some(&pm_token),
        Some(body),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "PM can assign to a managed employee: {created}");
    assert_eq!(created["created_by"], pm.id.to_string());
    assert_eq!(created["weight"], 3);

    // Cleanup: deleting the employee cascades the task + the manager link. We do
    // NOT delete the PM — it became an audit actor by creating a task, and the
    // audit-immutability trigger blocks the FK's ON DELETE SET NULL (hard-delete
    // of an audited user is intentionally impossible; that path uses soft-delete).
    users::delete(&pool, emp.id).await.unwrap();
}
