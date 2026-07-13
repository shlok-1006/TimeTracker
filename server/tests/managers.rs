//! Multi-manager assignment (user_managers): an employee can have several
//! managers, one, or none; PM scope checks consult the join table.
//! Hits a live DB via DATABASE_URL; skips if unset.

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

use server::db::users;
use server::role::UserRole;

async fn pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .ok()
}

async fn mk(pool: &PgPool, role: UserRole, tag: &str) -> users::UserSummary {
    users::create(
        pool,
        &format!("Mgr {tag}"),
        &format!("mgr-{tag}-{}@t.local", Uuid::new_v4()),
        "h",
        role,
        None,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn employee_can_have_many_one_or_no_managers() {
    let Some(pool) = pool().await else {
        eprintln!("skipping managers test: DATABASE_URL not set");
        return;
    };
    let pm1 = mk(&pool, UserRole::ProjectManager, "pm1").await;
    let pm2 = mk(&pool, UserRole::ProjectManager, "pm2").await;
    let pm3 = mk(&pool, UserRole::ProjectManager, "pm3").await;
    let emp = mk(&pool, UserRole::Employee, "emp").await;

    // MANY: both PM1 and PM2 manage the employee; PM3 does not.
    users::set_managers(&pool, emp.id, &[pm1.id, pm2.id])
        .await
        .unwrap();
    assert!(users::is_manager_of(&pool, pm1.id, emp.id).await.unwrap());
    assert!(users::is_manager_of(&pool, pm2.id, emp.id).await.unwrap());
    assert!(!users::is_manager_of(&pool, pm3.id, emp.id).await.unwrap());
    assert_eq!(users::managers_of(&pool, emp.id).await.unwrap().len(), 2);

    // ONE: replacing the set drops PM1.
    users::set_managers(&pool, emp.id, &[pm2.id]).await.unwrap();
    assert!(!users::is_manager_of(&pool, pm1.id, emp.id).await.unwrap());
    assert!(users::is_manager_of(&pool, pm2.id, emp.id).await.unwrap());

    // NONE: empty set clears all managers.
    users::set_managers(&pool, emp.id, &[]).await.unwrap();
    assert!(users::managers_of(&pool, emp.id).await.unwrap().is_empty());

    // add_manager (used on user creation) is idempotent.
    users::add_manager(&pool, emp.id, pm1.id).await.unwrap();
    users::add_manager(&pool, emp.id, pm1.id).await.unwrap();
    assert_eq!(users::managers_of(&pool, emp.id).await.unwrap().len(), 1);

    // Deleting a manager cascades their links away (FK ON DELETE CASCADE).
    users::delete(&pool, pm1.id).await.unwrap();
    assert!(users::managers_of(&pool, emp.id).await.unwrap().is_empty());

    for id in [pm2.id, pm3.id, emp.id] {
        users::delete(&pool, id).await.unwrap();
    }
}
