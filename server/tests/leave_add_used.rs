//! HR (or HRMS on its behalf) ADDS days to what someone has used, optionally with an idempotency key
//! (migration 0057, `POST /admin/leave/used` with `add_used_days`).
//!
//! Under test: adds accumulate; two adds at once both land; an approval that lands first doesn't cancel an add;
//! a negative add gives days back; an add can't go past the allotment or below approved requests; the same key
//! applies once and then replays; a key reused for anything else is refused (409 over HTTP); old clients sending
//! `used_days` get the same answer as before plus `"replayed": false`.
//! Needs a database: skipped when DATABASE_URL is unset. Uses year-2034 dates and a fresh user and type per test.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use chrono::NaiveDate;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use server::db::{leave, users};
use server::jwt::JwtKeys;
use server::leave_service;
use server::linear_service::LinearService;
use server::role::UserRole;
use server::storage::{S3Config, StorageClient};
use server::AppState;

const YEAR: i32 = 2034;
const SECRET: &str = "leave-add-used-test-secret";

async fn real_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .ok()
}

struct Fixture {
    hr: Uuid,
    user: Uuid,
    lt: Uuid,
}

/// An HR person, an employee and a fresh paid type with 12 days allotted.
async fn fixture(pool: &PgPool) -> Fixture {
    let tag = Uuid::new_v4().simple().to_string();
    let hr = users::create(
        pool,
        "hr-add",
        &format!("hr-add-{tag}@t.local"),
        "h",
        UserRole::Hr,
        None,
    )
    .await
    .unwrap();
    let user = users::create(
        pool,
        "add",
        &format!("add-{tag}@t.local"),
        "h",
        UserRole::Employee,
        None,
    )
    .await
    .unwrap();
    let lt = leave::create_type(
        pool,
        &format!("casual-add-{tag}"),
        true,
        12.0,
        12.0,
        12.0,
        &leave::TypeRules::default(),
    )
    .await
    .unwrap();
    Fixture {
        hr: hr.id,
        user: user.id,
        lt: lt.id,
    }
}

async fn balance_of(pool: &PgPool, f: &Fixture) -> leave::Balance {
    leave::balances(pool, f.user, YEAR)
        .await
        .unwrap()
        .into_iter()
        .find(|b| b.leave_type_id == f.lt)
        .expect("the test type is in the balance")
}

async fn add(pool: &PgPool, f: &Fixture, days: f64, key: Option<&str>) -> leave::SetUsed {
    leave::set_used_keyed(
        pool,
        f.user,
        f.lt,
        YEAR,
        leave::UsedTarget::Add(days),
        f.hr,
        key,
    )
    .await
    .unwrap()
}

async fn history_rows(pool: &PgPool, f: &Fixture) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM leave_used_history WHERE user_id = $1 AND leave_type_id = $2",
    )
    .bind(f.user)
    .bind(f.lt)
    .fetch_one(pool)
    .await
    .unwrap()
}

fn key() -> String {
    format!("hrms:short-day:{}", Uuid::new_v4().simple())
}

#[tokio::test]
async fn two_half_day_adds_make_one_day() {
    let Some(pool) = real_pool().await else {
        eprintln!("skipping leave-add test: DATABASE_URL not set");
        return;
    };
    let f = fixture(&pool).await;
    assert_eq!(
        add(&pool, &f, 0.5, None).await,
        leave::SetUsed::Saved {
            allotted: 12.0,
            approved: 0.0,
            adjustment: 0.5,
            used: 0.5,
            remaining: 11.5,
            changed: true
        }
    );
    add(&pool, &f, 0.5, None).await;
    let b = balance_of(&pool, &f).await;
    assert_eq!(
        (b.used_days, b.remaining_days, b.used_adjustment),
        (1.0, 11.0, 1.0)
    );
    assert_eq!(history_rows(&pool, &f).await, 2);
}

#[tokio::test]
async fn concurrent_adds_both_land() {
    let Some(pool) = real_pool().await else {
        eprintln!("skipping leave-add test: DATABASE_URL not set");
        return;
    };
    let f = fixture(&pool).await;
    for _ in 0..5 {
        let (a, b) = tokio::join!(add(&pool, &f, 0.5, None), add(&pool, &f, 0.5, None));
        assert!(matches!(a, leave::SetUsed::Saved { changed: true, .. }));
        assert!(matches!(b, leave::SetUsed::Saved { changed: true, .. }));
    }
    let b = balance_of(&pool, &f).await;
    assert_eq!((b.used_days, b.used_adjustment), (5.0, 5.0));
}

#[tokio::test]
async fn an_approval_before_the_add_does_not_cancel_it() {
    let Some(pool) = real_pool().await else {
        eprintln!("skipping leave-add test: DATABASE_URL not set");
        return;
    };
    let f = fixture(&pool).await;
    // HR recorded 2 days; HRMS read "used = 2" and is about to deduct half a day.
    leave::set_used(
        &pool,
        f.user,
        f.lt,
        YEAR,
        leave::UsedTarget::Used(2.0),
        f.hr,
    )
    .await
    .unwrap();
    // Meanwhile a 1-day request (2034-03-07 is a Tuesday) is approved.
    let (req, _) = leave_service::submit_request(
        &pool,
        f.user,
        f.lt,
        NaiveDate::from_ymd_opt(YEAR, 3, 7).unwrap(),
        NaiveDate::from_ymd_opt(YEAR, 3, 7).unwrap(),
        "",
        None,
        None,
    )
    .await
    .expect("a one-day request");
    assert!(leave_service::approve(&pool, req, f.hr).await.unwrap());
    // The add moves used from where it is now: 2 recorded + 1 approved + 0.5. Writing back the absolute
    // "2 + 0.5" would have stored 1.5 on top of the approval and lost the deduction.
    assert_eq!(
        add(&pool, &f, 0.5, None).await,
        leave::SetUsed::Saved {
            allotted: 12.0,
            approved: 1.0,
            adjustment: 2.5,
            used: 3.5,
            remaining: 8.5,
            changed: true
        }
    );
}

#[tokio::test]
async fn a_negative_add_gives_recorded_days_back_but_not_approved_ones() {
    let Some(pool) = real_pool().await else {
        eprintln!("skipping leave-add test: DATABASE_URL not set");
        return;
    };
    let f = fixture(&pool).await;
    add(&pool, &f, 1.5, None).await;
    let r = add(&pool, &f, -0.5, None).await;
    assert!(matches!(
        r,
        leave::SetUsed::Saved { adjustment, used, remaining, changed: true, .. }
            if adjustment == 1.0 && used == 1.0 && remaining == 11.0
    ));
    // Giving back more than HR recorded would eat into approved requests (none here, so below 0): refused.
    assert_eq!(
        add(&pool, &f, -1.5, None).await,
        leave::SetUsed::BelowApproved {
            allotted: 12.0,
            approved: 0.0
        }
    );
    // Giving back exactly what's recorded clears the adjustment row.
    add(&pool, &f, -1.0, None).await;
    let b = balance_of(&pool, &f).await;
    assert_eq!((b.used_days, b.used_adjustment), (0.0, 0.0));
}

#[tokio::test]
async fn an_add_beyond_the_allotment_is_refused() {
    let Some(pool) = real_pool().await else {
        eprintln!("skipping leave-add test: DATABASE_URL not set");
        return;
    };
    let f = fixture(&pool).await;
    add(&pool, &f, 11.5, None).await;
    assert_eq!(
        add(&pool, &f, 1.0, Some(&key())).await,
        leave::SetUsed::AboveAllotment { allotted: 12.0 }
    );
    // Up to the allotment exactly is fine.
    assert!(matches!(
        add(&pool, &f, 0.5, None).await,
        leave::SetUsed::Saved { used, remaining, .. } if used == 12.0 && remaining == 0.0
    ));
    // The allotment is lowered under the recorded days: adding is refused, giving back still works.
    leave::upsert_allocation(&pool, f.user, f.lt, YEAR, 10.0)
        .await
        .unwrap();
    assert_eq!(
        add(&pool, &f, 0.5, None).await,
        leave::SetUsed::AboveAllotment { allotted: 10.0 }
    );
    assert!(matches!(
        add(&pool, &f, -0.5, None).await,
        leave::SetUsed::Saved { used, .. } if used == 11.5
    ));
}

#[tokio::test]
async fn the_same_key_applies_once() {
    let Some(pool) = real_pool().await else {
        eprintln!("skipping leave-add test: DATABASE_URL not set");
        return;
    };
    let f = fixture(&pool).await;
    let k = key();
    assert!(matches!(
        add(&pool, &f, 0.5, Some(&k)).await,
        leave::SetUsed::Saved { used, changed: true, .. } if used == 0.5
    ));
    // The response was lost; HRMS retries.
    assert_eq!(
        add(&pool, &f, 0.5, Some(&k)).await,
        leave::SetUsed::Replayed {
            allotted: 12.0,
            approved: 0.0,
            adjustment: 0.5,
            used: 0.5,
            remaining: 11.5
        }
    );
    assert_eq!(balance_of(&pool, &f).await.used_days, 0.5);
    assert_eq!(history_rows(&pool, &f).await, 1);

    // Two retries racing each other: still once.
    let k2 = key();
    let (a, b) = tokio::join!(
        add(&pool, &f, 0.5, Some(&k2)),
        add(&pool, &f, 0.5, Some(&k2))
    );
    let saved = [&a, &b]
        .iter()
        .filter(|o| matches!(o, leave::SetUsed::Saved { .. }))
        .count();
    let replayed = [&a, &b]
        .iter()
        .filter(|o| matches!(o, leave::SetUsed::Replayed { .. }))
        .count();
    assert_eq!((saved, replayed), (1, 1), "{a:?} / {b:?}");
    assert_eq!(balance_of(&pool, &f).await.used_days, 1.0);

    // A keyed absolute target that changes nothing still remembers its key: after HR moves the balance, the
    // retry replays instead of putting it back.
    let k3 = key();
    assert!(matches!(
        leave::set_used_keyed(
            &pool,
            f.user,
            f.lt,
            YEAR,
            leave::UsedTarget::Used(1.0),
            f.hr,
            Some(&k3)
        )
        .await
        .unwrap(),
        leave::SetUsed::Saved { changed: false, .. }
    ));
    add(&pool, &f, 2.0, None).await;
    assert!(matches!(
        leave::set_used_keyed(&pool, f.user, f.lt, YEAR, leave::UsedTarget::Used(1.0), f.hr, Some(&k3)).await.unwrap(),
        leave::SetUsed::Replayed { used, .. } if used == 3.0
    ));
}

#[tokio::test]
async fn a_key_reused_for_something_else_is_refused() {
    let Some(pool) = real_pool().await else {
        eprintln!("skipping leave-add test: DATABASE_URL not set");
        return;
    };
    let f = fixture(&pool).await;
    let other = fixture(&pool).await;
    let k = key();
    add(&pool, &f, 0.5, Some(&k)).await;

    // Another person, another type, another year, another amount, another kind of request.
    assert_eq!(
        add(&pool, &other, 0.5, Some(&k)).await,
        leave::SetUsed::KeyConflict
    );
    let other_type = leave::set_used_keyed(
        &pool,
        f.user,
        other.lt,
        YEAR,
        leave::UsedTarget::Add(0.5),
        f.hr,
        Some(&k),
    )
    .await
    .unwrap();
    assert_eq!(other_type, leave::SetUsed::KeyConflict);
    let other_year = leave::set_used_keyed(
        &pool,
        f.user,
        f.lt,
        YEAR + 1,
        leave::UsedTarget::Add(0.5),
        f.hr,
        Some(&k),
    )
    .await
    .unwrap();
    assert_eq!(other_year, leave::SetUsed::KeyConflict);
    assert_eq!(
        add(&pool, &f, 1.0, Some(&k)).await,
        leave::SetUsed::KeyConflict
    );
    let absolute = leave::set_used_keyed(
        &pool,
        f.user,
        f.lt,
        YEAR,
        leave::UsedTarget::Used(0.5),
        f.hr,
        Some(&k),
    )
    .await
    .unwrap();
    assert_eq!(absolute, leave::SetUsed::KeyConflict);
    // Nothing moved.
    assert_eq!(balance_of(&pool, &f).await.used_days, 0.5);
    assert_eq!(balance_of(&pool, &other).await.used_days, 0.0);

    // The same new key for two different people at once (different locks): the unique index lets one in.
    let k2 = key();
    let (a, b) = tokio::join!(
        add(&pool, &f, 0.5, Some(&k2)),
        add(&pool, &other, 0.5, Some(&k2))
    );
    let conflicts = [&a, &b]
        .iter()
        .filter(|o| matches!(o, leave::SetUsed::KeyConflict))
        .count();
    assert_eq!(conflicts, 1, "{a:?} / {b:?}");
    let total = balance_of(&pool, &f).await.used_days + balance_of(&pool, &other).await.used_days;
    assert_eq!(total, 1.0, "exactly one of the two adds applied");

    // Deterministically: another transaction has stored the key but not committed when this call checks, so the
    // check passes; the insert then waits on the unique index, fails once that commits, and the whole call —
    // adjustment included — rolls back.
    let k3 = key();
    let before = balance_of(&pool, &f).await.used_days;
    let mut other_tx = pool.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO leave_used_history
             (user_id, leave_type_id, year, allotted_days, approved_days, old_used, new_used,
              idempotency_key, idempotency_request)
         VALUES ($1, $2, $3, 12, 0, 0, 0.5, $4, 'add_used_days=0.5')",
    )
    .bind(other.user)
    .bind(other.lt)
    .bind(YEAR)
    .bind(&k3)
    .execute(&mut *other_tx)
    .await
    .unwrap();
    let racing = {
        let pool = pool.clone();
        let (user, lt, hr, k3) = (f.user, f.lt, f.hr, k3.clone());
        tokio::spawn(async move {
            leave::set_used_keyed(
                &pool,
                user,
                lt,
                YEAR,
                leave::UsedTarget::Add(0.5),
                hr,
                Some(&k3),
            )
            .await
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(
        !racing.is_finished(),
        "the call is waiting on the unique index"
    );
    other_tx.commit().await.unwrap();
    assert_eq!(racing.await.unwrap().unwrap(), leave::SetUsed::KeyConflict);
    assert_eq!(balance_of(&pool, &f).await.used_days, before);
}

// ---- Over HTTP: the JSON contract, 400 / 409, and what old clients get. ----

fn app_with(pool: PgPool) -> Router {
    server::build_router(AppState::new(
        pool,
        JwtKeys::new(SECRET, 900),
        StorageClient::new(S3Config::insecure_local()),
        LinearService::from_env(),
        server::claude_provider::ClaudeProvider::from_env(),
        2_592_000,
    ))
}

async fn post(pool: &PgPool, as_user: Uuid, body: Value) -> (StatusCode, Value) {
    let tok = JwtKeys::new(SECRET, 900)
        .issue(as_user, UserRole::Hr, None, None)
        .unwrap();
    let req = Request::builder()
        .method("POST")
        .uri("/admin/leave/used")
        .header("authorization", format!("Bearer {tok}"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app_with(pool.clone()).oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn add_used_days_over_http() {
    let Some(pool) = real_pool().await else {
        eprintln!("skipping leave-add HTTP test: DATABASE_URL not set");
        return;
    };
    let f = fixture(&pool).await;
    let other = fixture(&pool).await;
    let k = key();
    let req = json!({
        "user_id": f.user, "leave_type_id": f.lt, "year": YEAR,
        "add_used_days": 0.5, "idempotency_key": k
    });

    let (s, first) = post(&pool, f.hr, req.clone()).await;
    assert_eq!(s, StatusCode::OK, "{first}");
    assert_eq!(
        first,
        json!({
            "user_id": f.user, "leave_type_id": f.lt, "year": YEAR,
            "allotted_days": 12.0, "used_days": 0.5, "remaining_days": 11.5,
            "approved_days": 0.0, "used_adjustment": 0.5, "replayed": false
        })
    );
    let (s, again) = post(&pool, f.hr, req.clone()).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(again["replayed"], json!(true));
    assert_eq!(again["used_days"], json!(0.5));

    // Same key, another person → 409, and nothing changes for them.
    let (s, err) = post(
        &pool,
        f.hr,
        json!({ "user_id": other.user, "leave_type_id": other.lt, "year": YEAR,
                "add_used_days": 0.5, "idempotency_key": k }),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{err}");
    assert_eq!(balance_of(&pool, &other).await.used_days, 0.0);

    // An old client sending used_days gets the same answer as before, plus "replayed": false.
    let (s, old) = post(
        &pool,
        f.hr,
        json!({ "user_id": f.user, "leave_type_id": f.lt, "year": YEAR, "used_days": 2 }),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{old}");
    assert_eq!(
        (old["used_days"].clone(), old["replayed"].clone()),
        (json!(2.0), json!(false))
    );

    // Past the allotment → 400.
    let (s, _) = post(
        &pool,
        f.hr,
        json!({ "user_id": f.user, "leave_type_id": f.lt, "year": YEAR, "add_used_days": 10.5 }),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Malformed requests → 400 before anything is touched.
    for bad in [
        json!({ "user_id": f.user, "leave_type_id": f.lt, "year": YEAR }),
        json!({ "user_id": f.user, "leave_type_id": f.lt, "year": YEAR, "used_days": 1, "add_used_days": 0.5 }),
        json!({ "user_id": f.user, "leave_type_id": f.lt, "year": YEAR, "add_used_days": 0 }),
        json!({ "user_id": f.user, "leave_type_id": f.lt, "year": YEAR, "add_used_days": 0.3 }),
        json!({ "user_id": f.user, "leave_type_id": f.lt, "year": YEAR, "add_used_days": 367 }),
        json!({ "user_id": f.user, "leave_type_id": f.lt, "year": YEAR, "add_used_days": 0.5, "idempotency_key": "" }),
        json!({ "user_id": f.user, "leave_type_id": f.lt, "year": YEAR, "add_used_days": 0.5, "idempotency_key": "has space" }),
    ] {
        let (s, e) = post(&pool, f.hr, bad.clone()).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad} → {e}");
    }
    assert_eq!(balance_of(&pool, &f).await.used_days, 2.0);

    // HR can't add to their own balance.
    let (s, _) = post(
        &pool,
        f.hr,
        json!({ "user_id": f.hr, "leave_type_id": f.lt, "year": YEAR, "add_used_days": 0.5 }),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}
