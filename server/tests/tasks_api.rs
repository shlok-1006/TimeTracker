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
    assert_eq!(
        s,
        StatusCode::OK,
        "PM can assign to a managed employee: {created}"
    );
    assert_eq!(created["created_by"], pm.id.to_string());
    assert_eq!(created["weight"], 3);

    // Cleanup: deleting the employee cascades the task + the manager link. We do
    // NOT delete the PM — it became an audit actor by creating a task, and the
    // audit-immutability trigger blocks the FK's ON DELETE SET NULL (hard-delete
    // of an audited user is intentionally impossible; that path uses soft-delete).
    users::delete(&pool, emp.id).await.unwrap();
}

// ───────────────────────── employee self-serve tasks ─────────────────────────

/// The ownership rule is the whole point of the self-serve routes, so it is
/// tested from the outside over real HTTP: an employee may add tasks for
/// themselves and manage those freely, may COMPLETE work a manager assigned,
/// but may never reword or delete assigned work — otherwise the list stops
/// being trustworthy for the manager relying on it.
#[tokio::test]
async fn employee_owns_their_own_tasks_but_not_assigned_ones() {
    let Some(pool) = real_pool().await else {
        eprintln!("skipping self-serve tasks round-trip: DATABASE_URL not set");
        return;
    };

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

    // A fresh employee with a known password so they can hold their own token.
    let tag = Uuid::new_v4();
    let email = format!("selftask-{tag}@t.local");
    let hash = server::auth::hash_password("ChangeMe!Emp1").unwrap();
    let emp = users::create(&pool, "Self Task", &email, &hash, UserRole::Employee, None)
        .await
        .unwrap();
    let (s, login) = send(
        app_with(pool.clone()),
        "POST",
        "/auth/login",
        None,
        Some(json!({ "email": email, "password": "ChangeMe!Emp1" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "employee login");
    let tok = login["access_token"].as_str().unwrap().to_string();

    // ── create one for myself: weight honoured, due date optional ──
    let (s, mine) = send(
        app_with(pool.clone()),
        "POST",
        "/me/tasks",
        Some(&tok),
        Some(json!({ "title": "  Draft the retro  ", "weight": 8, "due_date": "2026-08-20" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(mine["title"], "Draft the retro", "title is trimmed");
    assert_eq!(mine["weight"], 8);
    assert_eq!(mine["due_date"], "2026-08-20");
    assert_eq!(mine["status"], "open");
    assert_eq!(
        mine["created_by"], mine["user_id"],
        "a self-set task is authored by its owner — this is what later permits editing"
    );
    let mine_id = mine["id"].as_str().unwrap().to_string();

    // Due date really is optional.
    let (s, open_ended) = send(
        app_with(pool.clone()),
        "POST",
        "/me/tasks",
        Some(&tok),
        Some(json!({ "title": "Read the design doc" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert!(open_ended["due_date"].is_null(), "no date = open-ended");
    assert_eq!(open_ended["weight"], 5, "neutral default weight");

    // ── validation ──
    for bad in [json!({ "title": "x", "weight": 0 }), json!({ "title": "x", "weight": 11 })] {
        let (s, _) = send(app_with(pool.clone()), "POST", "/me/tasks", Some(&tok), Some(bad)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "weight must be 1..=10");
    }
    let (s, _) = send(
        app_with(pool.clone()),
        "POST",
        "/me/tasks",
        Some(&tok),
        Some(json!({ "title": "   " })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "blank title rejected");

    // ── HR assigns work to the same person ──
    let (s, assigned) = send(
        app_with(pool.clone()),
        "POST",
        &format!("/admin/users/{}/tasks", emp.id),
        Some(&hr),
        Some(json!({ "title": "Ship the payroll fix", "weight": 9 })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let assigned_id = assigned["id"].as_str().unwrap().to_string();

    // Completing assigned work is normal and allowed.
    let (s, done) = send(
        app_with(pool.clone()),
        "PATCH",
        &format!("/me/tasks/{assigned_id}"),
        Some(&tok),
        Some(json!({ "status": "done" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(done["status"], "done");

    // Rewording or deleting it is not.
    let (s, _) = send(
        app_with(pool.clone()),
        "PATCH",
        &format!("/me/tasks/{assigned_id}"),
        Some(&tok),
        Some(json!({ "title": "Something easier" })),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "cannot reword assigned work");
    let (s, _) = send(
        app_with(pool.clone()),
        "DELETE",
        &format!("/me/tasks/{assigned_id}"),
        Some(&tok),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "cannot delete assigned work");

    // ── editing my own, including taking a date back off ──
    let (s, edited) = send(
        app_with(pool.clone()),
        "PATCH",
        &format!("/me/tasks/{mine_id}"),
        Some(&tok),
        Some(json!({ "weight": 4 })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(edited["weight"], 4);
    assert_eq!(
        edited["due_date"], "2026-08-20",
        "an unrelated edit must not silently drop the due date"
    );

    let (s, cleared) = send(
        app_with(pool.clone()),
        "PATCH",
        &format!("/me/tasks/{mine_id}"),
        Some(&tok),
        Some(json!({ "clear_due_date": true })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        cleared["due_date"].is_null(),
        "clear_due_date makes it open-ended again — COALESCE alone cannot express this"
    );

    // ── one list holds both kinds ──
    let (s, list) = send(app_with(pool.clone()), "GET", "/me/tasks", Some(&tok), None).await;
    assert_eq!(s, StatusCode::OK);
    let titles: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["title"].as_str().unwrap())
        .collect();
    assert!(titles.contains(&"Draft the retro"));
    assert!(titles.contains(&"Ship the payroll fix"));

    // ── someone else's task is not found, not forbidden (no existence leak) ──
    let (s, other_login) = send(
        app_with(pool.clone()),
        "POST",
        "/auth/login",
        None,
        Some(json!({ "email": "employee@timetracker.local", "password": "ChangeMe!Emp1" })),
    )
    .await;
    if s == StatusCode::OK {
        let other = other_login["access_token"].as_str().unwrap();
        let (s, _) = send(
            app_with(pool.clone()),
            "PATCH",
            &format!("/me/tasks/{mine_id}"),
            Some(other),
            Some(json!({ "status": "done" })),
        )
        .await;
        assert_eq!(s, StatusCode::NOT_FOUND, "another person's task must 404");
    }

    // Deleting my own works.
    let (s, _) = send(
        app_with(pool.clone()),
        "DELETE",
        &format!("/me/tasks/{mine_id}"),
        Some(&tok),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    users::delete(&pool, emp.id).await.ok();
}
