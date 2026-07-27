//! Deleting a user who has audit history must succeed (the audit-immutability
//! trigger previously blocked the `ON DELETE SET NULL` on audit_logs.actor_id,
//! so HR couldn't delete anyone who had ever acted). Live DB; skips if unset.

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

use server::db::{audit, users};
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
async fn deleting_a_user_with_audit_history_succeeds() {
    let Some(pool) = pool().await else {
        eprintln!("skipping user-delete test: DATABASE_URL not set");
        return;
    };
    let tag = Uuid::new_v4();
    let u = users::create(
        &pool,
        "Del User",
        &format!("del-{tag}@t.local"),
        "h",
        UserRole::Employee,
        None,
    )
    .await
    .unwrap();

    // The user performs an audited action → an audit_logs row with actor_id = them.
    audit::log(&pool, u.id, "test.delete_probe", "user", Some(u.id)).await;

    // Before the fix this failed: the cascade's `SET NULL` on audit_logs.actor_id
    // hit the immutability trigger ("audit_logs are immutable").
    assert!(
        users::delete(&pool, u.id).await.unwrap(),
        "user with audit history should be deletable"
    );
    assert!(users::find_by_id(&pool, u.id).await.unwrap().is_none());

    // The audit row is retained (content immutable), with actor_id nulled.
    let row: (Option<Uuid>, String) = sqlx::query_as(
        "SELECT actor_id, action FROM audit_logs
         WHERE action = 'test.delete_probe' AND entity_id = $1",
    )
    .bind(u.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(row.0.is_none(), "actor_id should be nulled");
    assert_eq!(row.1, "test.delete_probe", "audit content preserved");
}
