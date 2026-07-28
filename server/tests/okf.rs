//! The OKF rulebook is seeded by migration 0037 and edited by HR via the repo.
//! This checks the seed is present and that an update stamps the editor and
//! content. Live DB; skips if DATABASE_URL is unset. Restores the seed after.

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
async fn okf_is_seeded_and_editable() {
    let Some(pool) = pool().await else {
        eprintln!("skipping okf test: DATABASE_URL not set");
        return;
    };

    // Seeded by migration 0037 from OKF.md.
    let seed = okf::get(&pool).await.unwrap();
    assert!(
        seed.content.contains("OKF"),
        "seed rulebook content should be present"
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

    // An edit stamps the editor and persists the new content.
    let marker = format!("\n<!-- edited {tag} -->\n");
    let updated = okf::update(&pool, &format!("{}{marker}", seed.content), hr.id)
        .await
        .unwrap();
    assert_eq!(updated.updated_by, Some(hr.id), "editor is recorded");
    assert!(updated.content.contains(&marker), "content is updated");

    // Restore the seed so re-runs start clean, then remove the test user.
    okf::update(&pool, &seed.content, hr.id).await.unwrap();
    users::delete(&pool, hr.id).await.ok();
}
