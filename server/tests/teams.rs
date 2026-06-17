//! Teams repository round-trip tests (Feature 4 Phase 1). Hits a live DB via
//! DATABASE_URL; skips if unset. Covers multi-team membership (the core
//! requirement: one employee in many teams).

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

use server::db::{teams, users};
use server::role::UserRole;

async fn pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    PgPoolOptions::new().max_connections(2).connect(&url).await.ok()
}

#[tokio::test]
async fn teams_membership_roundtrip() {
    let Some(pool) = pool().await else {
        eprintln!("skipping teams test: DATABASE_URL not set");
        return;
    };

    let tag = Uuid::new_v4();
    let emp = users::create(
        &pool, "Team Emp", &format!("teamemp-{tag}@t.local"), "h", UserRole::Employee, None,
    ).await.unwrap();

    // Two teams; the employee joins BOTH (multi-team).
    let alpha = teams::create(&pool, &format!("Alpha-{tag}"), "Alpha squad").await.unwrap();
    let beta = teams::create(&pool, &format!("Beta-{tag}"), "Beta squad").await.unwrap();

    // Duplicate name is rejected.
    assert!(teams::create(&pool, &format!("Alpha-{tag}"), "dup").await.is_err());

    teams::add_member(&pool, emp.id, alpha.id).await.unwrap();
    teams::add_member(&pool, emp.id, beta.id).await.unwrap();
    teams::add_member(&pool, emp.id, alpha.id).await.unwrap(); // idempotent

    // The employee belongs to BOTH teams.
    let mine = teams::teams_for_user(&pool, emp.id).await.unwrap();
    assert_eq!(mine.len(), 2, "employee should be in two teams");
    let names: Vec<&str> = mine.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&format!("Alpha-{tag}").as_str()));
    assert!(names.contains(&format!("Beta-{tag}").as_str()));

    // is_member + members_of.
    assert!(teams::is_member(&pool, emp.id, alpha.id).await.unwrap());
    let members = teams::members_of(&pool, alpha.id).await.unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].id, emp.id);

    // Remove from one team → left in one.
    assert!(teams::remove_member(&pool, emp.id, alpha.id).await.unwrap());
    assert!(!teams::is_member(&pool, emp.id, alpha.id).await.unwrap());
    assert_eq!(teams::teams_for_user(&pool, emp.id).await.unwrap().len(), 1);

    // get + list see the teams.
    assert!(teams::get(&pool, beta.id).await.unwrap().is_some());

    // Deleting a team cascades its memberships.
    assert!(teams::delete(&pool, beta.id).await.unwrap());
    assert!(teams::get(&pool, beta.id).await.unwrap().is_none());
    assert_eq!(teams::teams_for_user(&pool, emp.id).await.unwrap().len(), 0);

    // Cleanup.
    teams::delete(&pool, alpha.id).await.unwrap();
    users::delete(&pool, emp.id).await.unwrap();
}
