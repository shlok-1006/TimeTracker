//! HR sets how many days of a leave type someone has USED or has REMAINING (migrations 0053, 0054).
//!
//! "Used" is the sum of approved requests; `leave::set_used` records the difference HR adds on top (leave taken
//! outside TimeTracker), whichever of the two HR typed. Under test: the total HR sets is what the balance
//! reports, it can't go below what approved requests already account for, a later approved request adds on
//! top (so remaining keeps going down), setting it back to the approved total removes the adjustment, and the
//! "leaves left" roll-up follows the same arithmetic.
//! Needs a database: skipped when DATABASE_URL is unset. Uses year-2033 dates, a fresh user and type, and
//! cleans up after.

use chrono::NaiveDate;
use uuid::Uuid;

use server::db::{leave, users};
use server::leave_service;
use server::role::UserRole;

/// The tests here create company-wide leave types, and the "leaves left" assertions sum over every paid type —
/// so one test's type appearing mid-assertion in another would make it flaky. Run them one at a time.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn real_pool() -> Option<sqlx::PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .ok()
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

async fn balance_of(pool: &sqlx::PgPool, user: Uuid, lt: Uuid, year: i32) -> leave::Balance {
    leave::balances(pool, user, year)
        .await
        .unwrap()
        .into_iter()
        .find(|b| b.leave_type_id == lt)
        .expect("the test type is in the balance")
}

#[tokio::test]
async fn hr_sets_used_days_on_top_of_approved_requests() {
    let _serial = SERIAL.lock().await;
    let Some(pool) = real_pool().await else {
        eprintln!("skipping leave-used test: DATABASE_URL not set");
        return;
    };
    let tag = Uuid::new_v4().simple().to_string();
    let year = 2033;

    let hr = users::create(
        &pool,
        "hr-used",
        &format!("hr-used-{tag}@t.local"),
        "h",
        UserRole::Hr,
        None,
    )
    .await
    .unwrap();
    let user = users::create(
        &pool,
        "used",
        &format!("used-{tag}@t.local"),
        "h",
        UserRole::Employee,
        None,
    )
    .await
    .unwrap();
    let lt = leave::create_type(
        &pool,
        &format!("casual-used-{tag}"),
        true,
        12.0,
        12.0,
        12.0,
        &leave::TypeRules::default(),
    )
    .await
    .unwrap();

    // Nothing used yet.
    let b = balance_of(&pool, user.id, lt.id, year).await;
    assert_eq!(
        (
            b.allotted_days,
            b.used_days,
            b.remaining_days,
            b.used_adjustment
        ),
        (12.0, 0.0, 12.0, 0.0)
    );

    // HR records 3.5 days used outside TimeTracker.
    let r = leave::set_used(
        &pool,
        user.id,
        lt.id,
        year,
        leave::UsedTarget::Used(3.5),
        hr.id,
    )
    .await
    .unwrap();
    assert_eq!(
        r,
        leave::SetUsed::Saved {
            allotted: 12.0,
            approved: 0.0,
            adjustment: 3.5,
            used: 3.5,
            remaining: 8.5,
            changed: true
        }
    );
    let b = balance_of(&pool, user.id, lt.id, year).await;
    assert_eq!(
        (b.used_days, b.remaining_days, b.used_adjustment),
        (3.5, 8.5, 3.5)
    );

    // An approved 1-day request (2033-03-07 is a Monday) adds on top: 4.5 used.
    let (req, _) = leave_service::submit_request(
        &pool,
        user.id,
        lt.id,
        d(year, 3, 7),
        d(year, 3, 7),
        "",
        None,
        None,
    )
    .await
    .expect("a one-day request");
    assert!(leave::decide(&pool, req, "approved", hr.id).await.unwrap());
    let b = balance_of(&pool, user.id, lt.id, year).await;
    assert_eq!(
        (b.used_days, b.remaining_days, b.used_adjustment),
        (4.5, 7.5, 3.5)
    );

    // Can't go below what approved requests already account for.
    let r = leave::set_used(
        &pool,
        user.id,
        lt.id,
        year,
        leave::UsedTarget::Used(0.5),
        hr.id,
    )
    .await
    .unwrap();
    assert_eq!(
        r,
        leave::SetUsed::BelowApproved {
            allotted: 12.0,
            approved: 1.0
        }
    );
    assert_eq!(
        balance_of(&pool, user.id, lt.id, year).await.used_days,
        4.5,
        "a refused set changes nothing"
    );

    // Setting it to exactly the approved total removes the adjustment row.
    let r = leave::set_used(
        &pool,
        user.id,
        lt.id,
        year,
        leave::UsedTarget::Used(1.0),
        hr.id,
    )
    .await
    .unwrap();
    assert_eq!(
        r,
        leave::SetUsed::Saved {
            allotted: 12.0,
            approved: 1.0,
            adjustment: 0.0,
            used: 1.0,
            remaining: 11.0,
            changed: true
        }
    );
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM leave_used_adjustments WHERE user_id = $1")
            .bind(user.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(rows, 0, "a zero difference leaves no row behind");

    // The "leaves left" roll-up follows the same arithmetic (12 allotted − 1 approved − 2 recorded).
    leave::set_used(
        &pool,
        user.id,
        lt.id,
        year,
        leave::UsedTarget::Used(3.0),
        hr.id,
    )
    .await
    .unwrap();
    let left = leave::remaining_paid_by_user(&pool, year, None)
        .await
        .unwrap();
    let this_type_only = left.get(&user.id).copied().unwrap_or_default()
        - leave::balances(&pool, user.id, year)
            .await
            .unwrap()
            .iter()
            .filter(|b| b.paid && !b.special && b.leave_type_id != lt.id)
            .map(|b| b.remaining_days)
            .sum::<f64>();
    assert_eq!(this_type_only, 9.0);

    // Unknown user / type.
    assert_eq!(
        leave::set_used(
            &pool,
            Uuid::new_v4(),
            lt.id,
            year,
            leave::UsedTarget::Used(1.0),
            hr.id
        )
        .await
        .unwrap(),
        leave::SetUsed::NotFound
    );
    assert_eq!(
        leave::set_used(
            &pool,
            user.id,
            Uuid::new_v4(),
            year,
            leave::UsedTarget::Used(1.0),
            hr.id
        )
        .await
        .unwrap(),
        leave::SetUsed::NotFound
    );

    for id in [user.id, hr.id] {
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::query("DELETE FROM leave_types WHERE id = $1")
        .bind(lt.id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn used_is_capped_at_the_allotment_recorded_in_history_and_clamped_in_leaves_left() {
    let _serial = SERIAL.lock().await;
    let Some(pool) = real_pool().await else {
        eprintln!("skipping leave-used cap test: DATABASE_URL not set");
        return;
    };
    let tag = Uuid::new_v4().simple().to_string();
    let year = 2034;
    let hr = users::create(
        &pool,
        "hr-cap",
        &format!("hr-cap-{tag}@t.local"),
        "h",
        UserRole::Hr,
        None,
    )
    .await
    .unwrap();
    let user = users::create(
        &pool,
        "cap",
        &format!("cap-{tag}@t.local"),
        "h",
        UserRole::Employee,
        None,
    )
    .await
    .unwrap();
    let a = leave::create_type(
        &pool,
        &format!("cap-a-{tag}"),
        true,
        10.0,
        10.0,
        10.0,
        &leave::TypeRules::default(),
    )
    .await
    .unwrap();
    let b = leave::create_type(
        &pool,
        &format!("cap-b-{tag}"),
        true,
        5.0,
        5.0,
        5.0,
        &leave::TypeRules::default(),
    )
    .await
    .unwrap();

    // Above the allotment: refused, nothing written.
    assert_eq!(
        leave::set_used(
            &pool,
            user.id,
            a.id,
            year,
            leave::UsedTarget::Used(10.5),
            hr.id
        )
        .await
        .unwrap(),
        leave::SetUsed::AboveAllotment { allotted: 10.0 }
    );
    // Exactly the allotment is fine.
    leave::set_used(
        &pool,
        user.id,
        a.id,
        year,
        leave::UsedTarget::Used(10.0),
        hr.id,
    )
    .await
    .unwrap();
    leave::set_used(
        &pool,
        user.id,
        a.id,
        year,
        leave::UsedTarget::Used(4.0),
        hr.id,
    )
    .await
    .unwrap();
    // A save that changes nothing adds no history row.
    leave::set_used(
        &pool,
        user.id,
        a.id,
        year,
        leave::UsedTarget::Used(4.0),
        hr.id,
    )
    .await
    .unwrap();
    let history: Vec<(f64, f64)> = sqlx::query_as(
        "SELECT old_used, new_used FROM leave_used_history WHERE user_id = $1 AND leave_type_id = $2 ORDER BY created_at",
    )
    .bind(user.id)
    .bind(a.id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        history,
        vec![(0.0, 10.0), (10.0, 4.0)],
        "every change, old → new, nothing for a no-op"
    );

    // HR lowers type A's allotment below what's used (A: 2 − 4 = −2). "Leaves left" counts A as 0, so B's 5 days
    // are not eaten: 0 + 5, not −2 + 5.
    leave::upsert_allocation(&pool, user.id, a.id, year, 2.0)
        .await
        .unwrap();
    let left = leave::remaining_paid_by_user(&pool, year, None)
        .await
        .unwrap();
    let others: f64 = leave::balances(&pool, user.id, year)
        .await
        .unwrap()
        .iter()
        .filter(|x| x.paid && !x.special && x.leave_type_id != a.id && x.leave_type_id != b.id)
        .map(|x| x.remaining_days.max(0.0))
        .sum();
    assert_eq!(
        left.get(&user.id).copied().unwrap_or_default() - others,
        5.0
    );

    for id in [user.id, hr.id] {
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
    }
    for id in [a.id, b.id] {
        sqlx::query("DELETE FROM leave_types WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn hr_sets_remaining_and_approved_leave_keeps_lowering_it() {
    let _serial = SERIAL.lock().await;
    let Some(pool) = real_pool().await else {
        eprintln!("skipping leave-remaining test: DATABASE_URL not set");
        return;
    };
    let tag = Uuid::new_v4().simple().to_string();
    let year = 2035;
    let hr = users::create(
        &pool,
        "hr-rem",
        &format!("hr-rem-{tag}@t.local"),
        "h",
        UserRole::Hr,
        None,
    )
    .await
    .unwrap();
    let user = users::create(
        &pool,
        "rem",
        &format!("rem-{tag}@t.local"),
        "h",
        UserRole::Employee,
        None,
    )
    .await
    .unwrap();
    let lt = leave::create_type(
        &pool,
        &format!("earned-rem-{tag}"),
        true,
        11.5,
        11.5,
        11.5,
        &leave::TypeRules::default(),
    )
    .await
    .unwrap();

    // From the old records HR knows 8 days are left: 3.5 were used outside TimeTracker.
    let r = leave::set_used(
        &pool,
        user.id,
        lt.id,
        year,
        leave::UsedTarget::Remaining(8.0),
        hr.id,
    )
    .await
    .unwrap();
    assert_eq!(
        r,
        leave::SetUsed::Saved {
            allotted: 11.5,
            approved: 0.0,
            adjustment: 3.5,
            used: 3.5,
            remaining: 8.0,
            changed: true
        }
    );

    // The employee applies on the platform for 2 days (2035-03-05/06 are Mon/Tue) and it is approved through
    // the real approval path: remaining drops by exactly those 2 days.
    let (req, days) = leave_service::submit_request(
        &pool,
        user.id,
        lt.id,
        d(year, 3, 5),
        d(year, 3, 6),
        "",
        None,
        None,
    )
    .await
    .expect("a two-day request");
    assert_eq!(days, 2.0);
    assert!(leave_service::approve(&pool, req, hr.id).await.unwrap());
    let b = balance_of(&pool, user.id, lt.id, year).await;
    assert_eq!(
        (b.used_days, b.remaining_days, b.used_adjustment),
        (5.5, 6.0, 3.5)
    );

    // A half day too.
    let (req, _) = leave_service::submit_request(
        &pool,
        user.id,
        lt.id,
        d(year, 3, 12),
        d(year, 3, 12),
        "",
        Some(0.5),
        Some("first"),
    )
    .await
    .expect("a half-day request");
    assert!(leave_service::approve(&pool, req, hr.id).await.unwrap());
    assert_eq!(
        balance_of(&pool, user.id, lt.id, year).await.remaining_days,
        5.5
    );

    // More remaining than the allotment minus approved requests (11.5 − 2.5 = 9) is refused: those days were
    // taken on the platform, so only a bigger allotment can give them back.
    assert_eq!(
        leave::set_used(
            &pool,
            user.id,
            lt.id,
            year,
            leave::UsedTarget::Remaining(9.5),
            hr.id
        )
        .await
        .unwrap(),
        leave::SetUsed::BelowApproved {
            allotted: 11.5,
            approved: 2.5
        }
    );
    // Exactly 9 is allowed and clears what HR had recorded.
    let r = leave::set_used(
        &pool,
        user.id,
        lt.id,
        year,
        leave::UsedTarget::Remaining(9.0),
        hr.id,
    )
    .await
    .unwrap();
    assert!(matches!(
        r,
        leave::SetUsed::Saved {
            adjustment,
            remaining,
            ..
        } if adjustment == 0.0 && remaining == 9.0
    ));

    // Remaining 0: nothing left, so a new request is refused at submit.
    leave::set_used(
        &pool,
        user.id,
        lt.id,
        year,
        leave::UsedTarget::Remaining(0.0),
        hr.id,
    )
    .await
    .unwrap();
    assert_eq!(
        balance_of(&pool, user.id, lt.id, year).await.remaining_days,
        0.0
    );
    let refused = leave_service::submit_request(
        &pool,
        user.id,
        lt.id,
        d(year, 3, 19),
        d(year, 3, 19),
        "",
        None,
        None,
    )
    .await
    .expect_err("nothing left to take");
    assert!(
        refused.to_string().contains("insufficient balance"),
        "refused for the balance, not something else: {refused}"
    );

    // Saving the same remaining again writes nothing: no history row, and the recorded days stay attributed
    // to whoever set them (a second HR person re-saving can't take them over).
    let hr2 = users::create(
        &pool,
        "hr-rem2",
        &format!("hr-rem2-{tag}@t.local"),
        "h",
        UserRole::Hr,
        None,
    )
    .await
    .unwrap();
    let r = leave::set_used(
        &pool,
        user.id,
        lt.id,
        year,
        leave::UsedTarget::Remaining(0.0),
        hr2.id,
    )
    .await
    .unwrap();
    assert!(matches!(r, leave::SetUsed::Saved { changed: false, .. }));
    let set_by: Uuid = sqlx::query_scalar(
        "SELECT set_by FROM leave_used_adjustments WHERE user_id = $1 AND leave_type_id = $2",
    )
    .bind(user.id)
    .bind(lt.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(set_by, hr.id);

    // Over-drawn: the allotment is lowered to 2 while 2.5 days are approved. Nothing can remain, but
    // remaining 0 still clears the days HR recorded (remaining is then 2 − 2.5 = −0.5); anything above 0 is
    // refused.
    leave::upsert_allocation(&pool, user.id, lt.id, year, 2.0)
        .await
        .unwrap();
    assert_eq!(
        leave::set_used(
            &pool,
            user.id,
            lt.id,
            year,
            leave::UsedTarget::Remaining(0.5),
            hr.id
        )
        .await
        .unwrap(),
        leave::SetUsed::BelowApproved {
            allotted: 2.0,
            approved: 2.5
        }
    );
    assert_eq!(
        leave::set_used(
            &pool,
            user.id,
            lt.id,
            year,
            leave::UsedTarget::Remaining(0.0),
            hr.id
        )
        .await
        .unwrap(),
        leave::SetUsed::Saved {
            allotted: 2.0,
            approved: 2.5,
            adjustment: 0.0,
            used: 2.5,
            remaining: -0.5,
            changed: true
        }
    );

    // History keeps the allotment of the time with every change, and the stored total.
    let history: Vec<(f64, f64, f64)> = sqlx::query_as(
        "SELECT allotted_days, old_used, new_used FROM leave_used_history WHERE user_id = $1 ORDER BY created_at",
    )
    .bind(user.id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        history,
        vec![
            (11.5, 0.0, 3.5),
            (11.5, 6.0, 2.5),
            (11.5, 2.5, 11.5),
            (2.0, 11.5, 2.5)
        ]
    );

    sqlx::query("DELETE FROM leave_requests WHERE user_id = $1")
        .bind(user.id)
        .execute(&pool)
        .await
        .unwrap();
    for id in [user.id, hr.id, hr2.id] {
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::query("DELETE FROM leave_types WHERE id = $1")
        .bind(lt.id)
        .execute(&pool)
        .await
        .unwrap();
}

// ---- the route: who may call it, and what it accepts ----

mod route {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;
    use uuid::Uuid;

    use server::jwt::JwtKeys;
    use server::linear_service::LinearService;
    use server::role::UserRole;
    use server::storage::{S3Config, StorageClient};
    use server::AppState;

    const SECRET: &str = "leave-used-test-secret";

    fn app() -> axum::Router {
        let url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://localhost/timetracker".into());
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy(&url)
            .expect("lazy pool");
        server::build_router(AppState::new(
            pool,
            JwtKeys::new(SECRET, 900),
            StorageClient::new(S3Config::insecure_local()),
            LinearService::from_env(),
            server::claude_provider::ClaudeProvider::from_env(),
            2_592_000,
        ))
    }

    async fn post(caller: Uuid, role: UserRole, body: serde_json::Value) -> StatusCode {
        send("POST", "/admin/leave/used", caller, role, body).await
    }

    async fn send(
        method: &str,
        path: &str,
        caller: Uuid,
        role: UserRole,
        body: serde_json::Value,
    ) -> StatusCode {
        let token = JwtKeys::new(SECRET, 900)
            .issue(caller, role, None, None)
            .unwrap();
        app()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }

    fn body(user: Uuid, used: f64) -> serde_json::Value {
        serde_json::json!({ "user_id": user, "leave_type_id": Uuid::new_v4(), "year": 2034, "used_days": used })
    }

    #[tokio::test]
    async fn only_hr_and_admin_may_set_used_and_never_their_own() {
        if std::env::var("DATABASE_URL").is_err() {
            eprintln!("skipping leave-used route test: DATABASE_URL not set");
            return;
        }
        let someone = Uuid::new_v4();
        let me = Uuid::new_v4();
        assert_eq!(
            post(me, UserRole::Employee, body(someone, 1.0)).await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            post(me, UserRole::ProjectManager, body(someone, 1.0)).await,
            StatusCode::FORBIDDEN
        );
        // HR on their own record: refused before anything is looked up.
        assert_eq!(
            post(me, UserRole::Hr, body(me, 1.0)).await,
            StatusCode::BAD_REQUEST
        );
        // Bad values: refused (400) before any lookup.
        for v in [-1.0, 0.25, 366.5, 1000.0] {
            assert_eq!(
                post(me, UserRole::Hr, body(someone, v)).await,
                StatusCode::BAD_REQUEST,
                "used_days {v}"
            );
        }
        // remaining_days is validated the same way; exactly one of the two must be sent.
        let remaining = |v: f64| serde_json::json!({ "user_id": someone, "leave_type_id": Uuid::new_v4(), "year": 2034, "remaining_days": v });
        for v in [-0.5, 2.25, 400.0] {
            assert_eq!(
                post(me, UserRole::Hr, remaining(v)).await,
                StatusCode::BAD_REQUEST,
                "remaining_days {v}"
            );
        }
        let mut both = body(someone, 1.0);
        both["remaining_days"] = serde_json::json!(2.0);
        assert_eq!(post(me, UserRole::Hr, both).await, StatusCode::BAD_REQUEST);
        let mut neither = body(someone, 1.0);
        neither.as_object_mut().unwrap().remove("used_days");
        assert_eq!(
            post(me, UserRole::Hr, neither).await,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            post(me, UserRole::Hr, remaining(3.0)).await,
            StatusCode::NOT_FOUND
        );
        let mut bad_year = body(someone, 1.0);
        bad_year["year"] = serde_json::json!(1999);
        assert_eq!(
            post(me, UserRole::Hr, bad_year).await,
            StatusCode::BAD_REQUEST
        );
        // Valid input for a person / type that doesn't exist: 404 — HR and admin both get that far, and an
        // admin may target their own id (the self-edit rule doesn't apply to them).
        assert_eq!(
            post(me, UserRole::Hr, body(someone, 1.0)).await,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            post(me, UserRole::Admin, body(me, 1.0)).await,
            StatusCode::NOT_FOUND
        );

        // The allotment endpoints follow the same rule: HR can't raise, adjust or reset their OWN allotment
        // (that would undo the used/remaining rule by the back door), and an allotment is 0–366.
        let lt = Uuid::new_v4();
        let alloc = |u: Uuid, days: f64| serde_json::json!({ "user_id": u, "leave_type_id": lt, "year": 2034, "allotted_days": days });
        let adjust = |u: Uuid, delta: f64| serde_json::json!({ "user_id": u, "leave_type_id": lt, "year": 2034, "delta": delta });
        let reset =
            |u: Uuid| serde_json::json!({ "user_id": u, "leave_type_id": lt, "year": 2034 });
        for (method, path, b) in [
            ("POST", "/admin/leave/allocations", alloc(me, 30.0)),
            ("POST", "/admin/leave/allocations/adjust", adjust(me, 5.0)),
            ("DELETE", "/admin/leave/allocations", reset(me)),
            ("POST", "/admin/leave/allocations", alloc(someone, -3.0)),
            ("POST", "/admin/leave/allocations", alloc(someone, 400.0)),
        ] {
            assert_eq!(
                send(method, path, me, UserRole::Hr, b.clone()).await,
                StatusCode::BAD_REQUEST,
                "{method} {path} {b}"
            );
        }
        // An admin may reset their own (nothing there: 404, past the guard).
        assert_eq!(
            send(
                "DELETE",
                "/admin/leave/allocations",
                me,
                UserRole::Admin,
                reset(me)
            )
            .await,
            StatusCode::NOT_FOUND
        );
    }
}
