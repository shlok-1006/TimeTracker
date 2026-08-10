//! The onboarding sync's watermark (migration 0043).
//!
//! The sync runs on a timer and writing a person REPLACES every mapped column, so the only thing
//! standing between an hourly job and quietly reverting HR's corrections is `form_response_id`:
//! the sync writes a person only when their latest response is one it has not applied yet.
//!
//! Two behaviours make that work, and both are tested here against a live DB:
//!   1. the id round-trips — the sync stamps it, the directory listing hands it back, so the next
//!      run can tell "already applied" from "new";
//!   2. an HR edit (an upsert carrying NO id) LEAVES the stored id alone, so the corrected row
//!      still reads as applied and the next run walks past it.
//!
//! Without (2) the guard would be worse than useless: every manual correction would clear the
//! watermark and invite the very overwrite the watermark exists to prevent.

use chrono::Utc;
use uuid::Uuid;

use server::db::employee_directory as repo;

async fn real_pool() -> Option<sqlx::PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .ok()
}

/// A throwaway person to hang a profile on.
async fn seed_user(pool: &sqlx::PgPool) -> Uuid {
    let uid = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO users (id, name, email, password_hash, role)
         VALUES ($1, 'Form Source Test', $2, 'x', 'employee')",
        uid,
        format!("form-source-{uid}@test.local")
    )
    .execute(pool)
    .await
    .expect("seed user");
    uid
}

fn profile_with(source: Option<&str>) -> repo::EmployeeProfile {
    repo::EmployeeProfile {
        phone: Some("9000000000".into()),
        extra: serde_json::json!({}),
        form_response_id: source.map(str::to_string),
        form_submitted_at: source.map(|_| Utc::now()),
        ..Default::default()
    }
}

#[tokio::test]
async fn the_response_id_round_trips_to_the_directory_row() {
    let Some(pool) = real_pool().await else {
        eprintln!("DATABASE_URL unset — skipping directory watermark test");
        return;
    };
    let uid = seed_user(&pool).await;

    repo::upsert_profile(&pool, uid, &profile_with(Some("RESP_A")))
        .await
        .expect("first sync write");

    // The listing carries it, not just the bundle: the scheduled sync reads the whole roster once
    // and decides from that, so an id visible only on the detail endpoint would be no use.
    let entry = repo::get_entry(&pool, uid)
        .await
        .expect("entry read")
        .expect("entry exists");
    assert_eq!(entry.form_response_id.as_deref(), Some("RESP_A"));
    assert!(entry.has_profile);

    let bundle = repo::get_bundle(&pool, uid)
        .await
        .expect("bundle read")
        .expect("bundle exists");
    let p = bundle.profile.expect("profile exists");
    assert_eq!(p.form_response_id.as_deref(), Some("RESP_A"));
    assert!(p.form_submitted_at.is_some());

    sqlx::query!("DELETE FROM users WHERE id = $1", uid)
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
async fn an_hr_edit_keeps_the_watermark_so_the_correction_sticks() {
    let Some(pool) = real_pool().await else {
        eprintln!("DATABASE_URL unset — skipping HR-edit watermark test");
        return;
    };
    let uid = seed_user(&pool).await;

    repo::upsert_profile(&pool, uid, &profile_with(Some("RESP_A")))
        .await
        .expect("sync write");

    // HR fixes the phone number through PUT /admin/directory/:id. That payload has no response
    // id — it is a human editing a form, not the form speaking.
    let mut edit = profile_with(None);
    edit.phone = Some("9111111111".into());
    repo::upsert_profile(&pool, uid, &edit)
        .await
        .expect("HR edit");

    let entry = repo::get_entry(&pool, uid)
        .await
        .expect("entry read")
        .expect("entry exists");
    assert_eq!(
        entry.form_response_id.as_deref(),
        Some("RESP_A"),
        "an HR edit must not blank the watermark, or the next scheduled run would \
         treat the row as unsynced and overwrite the correction"
    );

    let p = repo::get_bundle(&pool, uid)
        .await
        .expect("bundle read")
        .expect("bundle exists")
        .profile
        .expect("profile exists");
    assert_eq!(p.phone.as_deref(), Some("9111111111"), "the edit landed");

    sqlx::query!("DELETE FROM users WHERE id = $1", uid)
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
async fn a_re_submission_moves_the_watermark() {
    let Some(pool) = real_pool().await else {
        eprintln!("DATABASE_URL unset — skipping re-submission test");
        return;
    };
    let uid = seed_user(&pool).await;

    repo::upsert_profile(&pool, uid, &profile_with(Some("RESP_A")))
        .await
        .expect("first write");
    // Someone fills the form again to correct themselves: a new id, and the form should win.
    repo::upsert_profile(&pool, uid, &profile_with(Some("RESP_B")))
        .await
        .expect("second write");

    let entry = repo::get_entry(&pool, uid)
        .await
        .expect("entry read")
        .expect("entry exists");
    assert_eq!(entry.form_response_id.as_deref(), Some("RESP_B"));

    sqlx::query!("DELETE FROM users WHERE id = $1", uid)
        .execute(&pool)
        .await
        .ok();
}
