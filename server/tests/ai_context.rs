//! Unified AI context test (Feature 5 Phase 3): open manual tasks are merged
//! into the analyzer context as `task:<uuid>`; done tasks are excluded. Hits a
//! live DB via DATABASE_URL; skips if unset.

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

use server::analysis_service;
use server::db::{manual_tasks, users};
use server::linear_service::LinearService;
use server::role::UserRole;

async fn pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    PgPoolOptions::new().max_connections(2).connect(&url).await.ok()
}

#[tokio::test]
async fn open_manual_tasks_appear_in_context() {
    let Some(pool) = pool().await else {
        eprintln!("skipping ai_context test: DATABASE_URL not set");
        return;
    };
    let tag = Uuid::new_v4();
    // A temp employee whose email won't match Linear → Linear part is empty,
    // so the context is exactly our manual tasks.
    let emp = users::create(&pool, "Ctx Emp", &format!("ctx-{tag}@nolinear.local"), "h", UserRole::Employee, None)
        .await.unwrap();
    let pm = users::create(&pool, "Ctx PM", &format!("ctxpm-{tag}@nolinear.local"), "h", UserRole::ProjectManager, None)
        .await.unwrap();

    let open = manual_tasks::create(&pool, emp.id, pm.id, "Open task", "do this").await.unwrap();
    let done = manual_tasks::create(&pool, emp.id, pm.id, "Done task", "already finished").await.unwrap();
    manual_tasks::set_status(&pool, done.id, "done").await.unwrap();

    let linear = LinearService::from_env();
    let ctx = analysis_service::build_context(&pool, &linear, emp.id).await.unwrap();

    let ids: Vec<&str> = ctx.iter().map(|t| t.id.as_str()).collect();
    assert!(ids.contains(&format!("task:{}", open.id).as_str()), "open task should be in context");
    assert!(!ids.contains(&format!("task:{}", done.id).as_str()), "done task must be excluded");

    let entry = ctx.iter().find(|t| t.id == format!("task:{}", open.id)).unwrap();
    assert_eq!(entry.title, "Open task");
    assert!(entry.labels.contains(&"manual task".to_string()));

    users::delete(&pool, emp.id).await.unwrap();
    users::delete(&pool, pm.id).await.unwrap();
}
