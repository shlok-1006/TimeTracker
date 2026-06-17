//! Onboarding repository round-trip (Feature 6A). Hits a live DB via
//! DATABASE_URL; skips if unset.

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

use server::db::{onboarding, users};
use server::role::UserRole;

async fn pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    PgPoolOptions::new().max_connections(2).connect(&url).await.ok()
}

#[tokio::test]
async fn onboarding_pipeline_roundtrip() {
    let Some(pool) = pool().await else {
        eprintln!("skipping onboarding test: DATABASE_URL not set");
        return;
    };
    let tag = Uuid::new_v4();
    let hr = users::create(
        &pool, "HR", &format!("hr-onb-{tag}@t.local"), "h", UserRole::Hr, None,
    ).await.unwrap();

    // Stages are seeded by the migration; the pipeline must be non-empty.
    let stages = onboarding::list_stages(&pool).await.unwrap();
    assert!(stages.len() >= 2, "expected seeded stages");
    let first = stages.first().unwrap();
    let second = &stages[1];

    // Create defaults to the first pipeline stage.
    let first_id = onboarding::first_stage_id(&pool).await.unwrap();
    assert_eq!(first_id, first.id);
    let cand = onboarding::create(
        &pool,
        "Ada Lovelace",
        &format!("ada-{tag}@candidate.local"),
        "Engineer",
        first_id,
        hr.id,
    )
    .await
    .unwrap();
    assert_eq!(cand.stage_name, first.name);
    assert_eq!(cand.status, "active");

    // Appears in the Kanban feed.
    let listed = onboarding::list(&pool).await.unwrap();
    assert!(listed.iter().any(|c| c.id == cand.id));

    // Stage transition.
    assert!(onboarding::set_stage(&pool, cand.id, second.id).await.unwrap());
    let moved = onboarding::get(&pool, cand.id).await.unwrap().unwrap();
    assert_eq!(moved.stage_id, second.id);
    assert_eq!(moved.stage_name, second.name);

    // Update fields (COALESCE leaves email untouched).
    assert!(onboarding::update(&pool, cand.id, Some("Ada L."), None, None).await.unwrap());
    let updated = onboarding::get(&pool, cand.id).await.unwrap().unwrap();
    assert_eq!(updated.name, "Ada L.");
    assert_eq!(updated.email, cand.email);

    // Checklist task: create -> toggle done -> delete.
    let task = onboarding::create_task(&pool, cand.id, "Sign offer letter").await.unwrap();
    assert!(!task.done);
    assert!(onboarding::set_task_done(&pool, task.id, true).await.unwrap());
    let tasks = onboarding::list_tasks(&pool, cand.id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].done);
    assert!(tasks[0].done_at.is_some());
    assert!(onboarding::delete_task(&pool, task.id).await.unwrap());
    assert!(onboarding::list_tasks(&pool, cand.id).await.unwrap().is_empty());

    // Document metadata.
    let key = format!("candidates/{}/{}-resume.pdf", cand.id, Uuid::new_v4());
    let doc = onboarding::add_document(&pool, cand.id, "resume", &key).await.unwrap();
    assert_eq!(doc.doc_type, "resume");
    let docs = onboarding::list_documents(&pool, cand.id).await.unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].storage_key, key);

    // Convert -> employee user; candidate flagged hired in the final stage.
    let new_user = users::create(
        &pool,
        &updated.name,
        &format!("ada-emp-{tag}@t.local"),
        "h",
        UserRole::Employee,
        None,
    )
    .await
    .unwrap();
    let final_stage = stages.last().unwrap();
    onboarding::mark_converted(&pool, cand.id, new_user.id, final_stage.id).await.unwrap();
    let hired = onboarding::get(&pool, cand.id).await.unwrap().unwrap();
    assert_eq!(hired.status, "hired");
    assert_eq!(hired.converted_user_id, Some(new_user.id));
    assert_eq!(hired.stage_id, final_stage.id);
    assert!(hired.hired_at.is_some());

    // Delete cascades tasks + documents.
    assert!(onboarding::delete(&pool, cand.id).await.unwrap());
    assert!(onboarding::get(&pool, cand.id).await.unwrap().is_none());

    // Cleanup.
    users::delete(&pool, new_user.id).await.unwrap();
    users::delete(&pool, hr.id).await.unwrap();
}
