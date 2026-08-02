//! The OKF policy library (migration 0038) seeds the handbook and lets HR
//! create/edit/delete documents. Live DB; skips if DATABASE_URL is unset.

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

use server::db::{okf, users};
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
async fn policy_library_is_seeded_and_crud_works() {
    let Some(pool) = pool().await else {
        eprintln!("skipping okf test: DATABASE_URL not set");
        return;
    };

    // Seeded by migration 0038: many documents, including the system rulebook.
    let all = okf::list(&pool).await.unwrap();
    assert!(all.len() >= 10, "handbook should be seeded");
    assert!(
        all.iter().any(|d| d.slug == okf::SYSTEM_SLUG),
        "system rulebook present"
    );

    let tag = Uuid::new_v4();
    let hr = users::create(
        &pool,
        "OKF Editor",
        &format!("okf-{tag}@t.local"),
        "h",
        UserRole::Hr,
        None,
    )
    .await
    .unwrap();

    // Create → edit → read back → delete.
    let doc = okf::create(&pool, "Test Policy", "Testing", "# Test\n\nbody", hr.id)
        .await
        .unwrap();
    assert_eq!(doc.kind, "markdown");

    let edited = okf::update(
        &pool,
        doc.id,
        "Test Policy v2",
        "Testing",
        "# v2",
        None,
        hr.id,
    )
    .await
    .unwrap();
    assert_eq!(edited.title, "Test Policy v2");
    assert_eq!(edited.content, "# v2");
    assert_eq!(edited.updated_by, Some(hr.id));

    let fetched = okf::get(&pool, doc.id).await.unwrap();
    assert_eq!(fetched.content, "# v2");

    okf::delete(&pool, doc.id).await.unwrap();
    assert!(okf::get(&pool, doc.id).await.is_err(), "deleted");

    users::delete(&pool, hr.id).await.ok();
}
