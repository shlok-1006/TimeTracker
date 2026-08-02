//! Grace-time grants fold into the weekly hours summary (Feature: manual time).
//! Hits a live DB via DATABASE_URL; skips if unset.

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

use server::db::{intervals, time_grants, users};
use server::role::UserRole;

async fn pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .ok()
}

#[tokio::test]
async fn grace_grants_add_to_week_total_and_revert() {
    let Some(pool) = pool().await else {
        eprintln!("skipping grace test: DATABASE_URL not set");
        return;
    };
    let tag = Uuid::new_v4();
    let emp = users::create(
        &pool,
        "Grace Emp",
        &format!("grace-{tag}@t.local"),
        "h",
        UserRole::Employee,
        None,
    )
    .await
    .unwrap();

    // Baseline: no grace, whatever the (likely zero) intervals give.
    let before = intervals::hours_summary(&pool, emp.id).await.unwrap();
    assert_eq!(before.week_grace_seconds, 0);
    let base_week = before.week_seconds;

    // Grant 2h to the current week; it lands on the same week the summary scopes.
    let wk = time_grants::current_week_start(&pool, emp.id)
        .await
        .unwrap();
    let first = time_grants::create(&pool, emp.id, wk, 2 * 3600, "off-tracker work", emp.id)
        .await
        .unwrap();

    let after = intervals::hours_summary(&pool, emp.id).await.unwrap();
    assert_eq!(after.week_grace_seconds, 2 * 3600);
    assert_eq!(after.week_seconds, base_week + 2 * 3600);

    // A second grant accumulates.
    time_grants::create(&pool, emp.id, wk, 30 * 60, "extra", emp.id)
        .await
        .unwrap();
    let after2 = intervals::hours_summary(&pool, emp.id).await.unwrap();
    assert_eq!(after2.week_grace_seconds, 2 * 3600 + 30 * 60);

    // Deleting the first grant reverts by exactly its amount.
    assert!(time_grants::delete(&pool, first.id).await.unwrap());
    let after3 = intervals::hours_summary(&pool, emp.id).await.unwrap();
    assert_eq!(after3.week_grace_seconds, 30 * 60);
    assert_eq!(after3.week_seconds, base_week + 30 * 60);

    // Cleanup (deleting the user cascades their grants).
    users::delete(&pool, emp.id).await.unwrap();
}
