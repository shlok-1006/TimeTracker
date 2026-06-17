//! Manual-tasks repository round-trip (Feature 5 Phase 1). Hits a live DB via
//! DATABASE_URL; skips if unset.

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

use server::db::{manual_tasks, users};
use server::role::UserRole;

async fn pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    PgPoolOptions::new().max_connections(2).connect(&url).await.ok()
}

#[tokio::test]
async fn manual_task_crud_roundtrip() {
    let Some(pool) = pool().await else {
        eprintln!("skipping manual_tasks test: DATABASE_URL not set");
        return;
    };
    let tag = Uuid::new_v4();
    let pm = users::create(
        &pool, "PM", &format!("pm-mt-{tag}@t.local"), "h", UserRole::ProjectManager, None,
    ).await.unwrap();
    let emp = users::create(
        &pool, "Emp", &format!("emp-mt-{tag}@t.local"), "h", UserRole::Employee, None,
    ).await.unwrap();

    // Create.
    let task = manual_tasks::create(&pool, emp.id, pm.id, "Write API docs", "Cover all endpoints")
        .await
        .unwrap();
    assert_eq!(task.user_id, emp.id);
    assert_eq!(task.created_by, Some(pm.id));
    assert_eq!(task.status, "open");
    assert_eq!(task.title, "Write API docs");

    // List + get.
    let list = manual_tasks::list_for_user(&pool, emp.id).await.unwrap();
    assert_eq!(list.len(), 1);
    let got = manual_tasks::get(&pool, task.id).await.unwrap().unwrap();
    assert_eq!(got.description, "Cover all endpoints");

    // Update title only; description preserved.
    assert!(manual_tasks::update(&pool, task.id, Some("Write & publish API docs"), None).await.unwrap());
    let after = manual_tasks::get(&pool, task.id).await.unwrap().unwrap();
    assert_eq!(after.title, "Write & publish API docs");
    assert_eq!(after.description, "Cover all endpoints");
    assert!(after.updated_at >= after.created_at);

    // Mark done.
    assert!(manual_tasks::set_status(&pool, task.id, "done").await.unwrap());
    assert_eq!(manual_tasks::get(&pool, task.id).await.unwrap().unwrap().status, "done");

    // Delete.
    assert!(manual_tasks::delete(&pool, task.id).await.unwrap());
    assert!(manual_tasks::get(&pool, task.id).await.unwrap().is_none());
    assert!(manual_tasks::list_for_user(&pool, emp.id).await.unwrap().is_empty());

    // Cleanup (deleting the assignee would also cascade any remaining tasks).
    users::delete(&pool, emp.id).await.unwrap();
    users::delete(&pool, pm.id).await.unwrap();
}
