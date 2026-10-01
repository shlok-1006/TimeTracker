//! Paternity / maternity eligibility (migration 0050), calendar-day counting, half-days (incl. the lower
//! bound), overlap guard, HR overrides and the "adjust" path, the locked approval re-check (incl. two
//! approvals at once), and the "leaves left" totals excluding special types.
//! Needs a database: skipped when DATABASE_URL is unset. Uses year-2031 dates, fresh users/types, and
//! deletes everything it created.

use chrono::{Datelike, NaiveDate};
use uuid::Uuid;

use server::db::{leave, users};
use server::leave_service;
use server::role::UserRole;

async fn real_pool() -> Option<sqlx::PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(6)
        .connect(&url)
        .await
        .ok()
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

async fn person(
    pool: &sqlx::PgPool,
    made: &mut Vec<Uuid>,
    tag: &str,
    gender: Option<&str>,
    joined: Option<NaiveDate>,
) -> Uuid {
    let u = users::create(
        pool,
        tag,
        &format!("{tag}-{}@t.local", Uuid::new_v4()),
        "h",
        UserRole::Employee,
        None,
    )
    .await
    .unwrap();
    sqlx::query("UPDATE users SET joined_on = $2 WHERE id = $1")
        .bind(u.id)
        .bind(joined)
        .execute(pool)
        .await
        .unwrap();
    if let Some(g) = gender {
        sqlx::query(
            "INSERT INTO employee_profiles (user_id, gender) VALUES ($1, $2)
             ON CONFLICT (user_id) DO UPDATE SET gender = EXCLUDED.gender",
        )
        .bind(u.id)
        .bind(g)
        .execute(pool)
        .await
        .unwrap();
    }
    made.push(u.id);
    u.id
}

async fn bal(pool: &sqlx::PgPool, uid: Uuid, lt: Uuid, as_of: NaiveDate) -> leave::Balance {
    leave::balances_as_of(pool, uid, as_of.year(), as_of)
        .await
        .unwrap()
        .into_iter()
        .find(|b| b.leave_type_id == lt)
        .expect("type present")
}

fn err_text<T>(r: Result<T, server::error::AppError>) -> String {
    match r {
        Ok(_) => "OK".into(),
        Err(e) => format!("{e:?}"),
    }
}

async fn cleanup(pool: &sqlx::PgPool, users: &[Uuid], types: &[Uuid]) {
    // users cascade to their requests, allocations and profiles; then the types are free to go
    sqlx::query("DELETE FROM users WHERE id = ANY($1)")
        .bind(users)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM leave_types WHERE id = ANY($1)")
        .bind(types)
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn parental_eligibility_day_basis_and_approval() {
    let Some(pool) = real_pool().await else {
        eprintln!("skipping parental leave test: DATABASE_URL not set");
        return;
    };
    let mut made = Vec::new();
    let tag = Uuid::new_v4().simple().to_string();
    let rule = |g: &str, basis: &str| leave::TypeRules {
        eligible_gender: Some(g.into()),
        min_tenure_months: 12,
        day_basis: basis.into(),
    };
    let pat = leave::create_type(
        &pool,
        &format!("pat-{tag}"),
        true,
        10.0,
        10.0,
        10.0,
        &rule("male", "working"),
    )
    .await
    .unwrap();
    let mat = leave::create_type(
        &pool,
        &format!("mat-{tag}"),
        true,
        180.0,
        180.0,
        180.0,
        &rule("female", "calendar"),
    )
    .await
    .unwrap();
    let types = [pat.id, mat.id];
    let mon = d(2031, 3, 3); // a Monday

    // ── eligibility by gender (case-insensitive, short forms, stray whitespace) and tenure ──
    let veteran_m = person(&pool, &mut made, "vm", Some("Male"), Some(d(2029, 1, 1))).await;
    let veteran_f = person(
        &pool,
        &mut made,
        "vf",
        Some(" female\t"),
        Some(d(2029, 1, 1)),
    )
    .await;
    let short_f = person(&pool, &mut made, "sf", Some("F"), Some(d(2029, 1, 1))).await;
    let newbie_m = person(&pool, &mut made, "nm", Some("male"), Some(d(2030, 9, 1))).await;
    let no_gender = person(&pool, &mut made, "ng", None, Some(d(2029, 1, 1))).await;
    let no_join = person(&pool, &mut made, "nj", Some("Male"), None).await;

    let b = bal(&pool, veteran_m, pat.id, mon).await;
    assert!(b.eligible && b.allotted_days == 10.0 && b.special && b.day_basis == "working");
    let b = bal(&pool, veteran_m, mat.id, mon).await;
    assert!(
        !b.eligible && b.allotted_days == 0.0,
        "a man gets no maternity allotment"
    );
    assert!(b.eligibility_note.as_deref().unwrap().contains("female"));
    assert_eq!(
        b.eligible_from, None,
        "a gender mismatch is never 'eligible from' a date"
    );
    assert_eq!(
        bal(&pool, veteran_f, mat.id, mon).await.allotted_days,
        180.0,
        "tab/space-padded gender"
    );
    assert_eq!(
        bal(&pool, short_f, mat.id, mon).await.allotted_days,
        180.0,
        "\"F\" counts as female"
    );
    assert_eq!(bal(&pool, veteran_f, pat.id, mon).await.allotted_days, 0.0);
    let b = bal(&pool, newbie_m, pat.id, mon).await;
    assert!(!b.eligible, "under a year of service");
    assert_eq!(
        b.eligible_from,
        Some(d(2031, 9, 1)),
        "tenure-only gap carries the date"
    );
    assert!(
        b.eligibility_note
            .as_deref()
            .unwrap()
            .contains("1 Sep 2031"),
        "{:?}",
        b.eligibility_note
    );
    assert!(
        !bal(&pool, no_gender, pat.id, mon).await.eligible,
        "no gender on record → not eligible"
    );
    let b = bal(&pool, no_join, pat.id, mon).await;
    assert!(
        !b.eligible
            && b.eligibility_note
                .as_deref()
                .unwrap()
                .contains("joining date")
    );
    assert_eq!(b.eligible_from, None);

    // ── eligibility is judged on the leave's START date ──
    let early = leave_service::submit_request(
        &pool,
        newbie_m,
        pat.id,
        d(2031, 8, 25),
        d(2031, 8, 26),
        "",
        None,
    )
    .await;
    assert!(
        err_text(early).contains("isn't available"),
        "refused for eligibility, not something else"
    );
    let ok = leave_service::submit_request(
        &pool,
        newbie_m,
        pat.id,
        d(2031, 9, 1),
        d(2031, 9, 2),
        "",
        None,
    )
    .await
    .expect("starts on the 1-year mark");
    assert_eq!(ok.1, 2.0);

    // ── calendar basis: a Mon..Sun week is 7 days of maternity; 180 fits, 181 doesn't; no fractions ──
    let (_, wk) =
        leave_service::submit_request(&pool, veteran_f, mat.id, mon, d(2031, 3, 9), "", None)
            .await
            .unwrap();
    assert_eq!(wk, 7.0, "weekends inside maternity count");
    let other_f = person(&pool, &mut made, "of", Some("Female"), Some(d(2029, 1, 1))).await;
    let start = d(2031, 1, 1);
    let (_, full) = leave_service::submit_request(
        &pool,
        other_f,
        mat.id,
        start,
        start + chrono::Duration::days(179),
        "",
        None,
    )
    .await
    .unwrap();
    assert_eq!(full, 180.0);
    let third_f = person(&pool, &mut made, "tf", Some("Female"), Some(d(2029, 1, 1))).await;
    assert!(
        err_text(
            leave_service::submit_request(
                &pool,
                third_f,
                mat.id,
                start,
                start + chrono::Duration::days(180),
                "",
                None
            )
            .await
        )
        .contains("insufficient"),
        "181 calendar days exceeds 180"
    );
    assert!(
        err_text(
            leave_service::submit_request(
                &pool,
                third_f,
                mat.id,
                d(2031, 6, 2),
                d(2031, 6, 8),
                "",
                Some(6.5)
            )
            .await
        )
        .contains("whole calendar days"),
        "no half-days of maternity"
    );

    // ── half-days: 0.5 ok, 0.25 refused, more than the span refused, and AT MOST half a day shaved ──
    let (_, half) =
        leave_service::submit_request(&pool, veteran_m, pat.id, mon, mon, "", Some(0.5))
            .await
            .unwrap();
    assert_eq!(half, 0.5);
    assert!(err_text(
        leave_service::submit_request(
            &pool,
            veteran_m,
            pat.id,
            d(2031, 3, 4),
            d(2031, 3, 4),
            "",
            Some(0.25)
        )
        .await
    )
    .contains("multiple of 0.5"));
    assert!(err_text(
        leave_service::submit_request(
            &pool,
            veteran_m,
            pat.id,
            d(2031, 3, 4),
            d(2031, 3, 4),
            "",
            Some(1.5)
        )
        .await
    )
    .contains("exceeds"));
    let wk2 = (d(2031, 3, 10), d(2031, 3, 14)); // Mon..Fri = 5 working days
    assert!(
        err_text(
            leave_service::submit_request(&pool, veteran_m, pat.id, wk2.0, wk2.1, "", Some(0.5))
                .await
        )
        .contains("can be booked as"),
        "a week off can't be charged as half a day"
    );
    let (_, four_half) =
        leave_service::submit_request(&pool, veteran_m, pat.id, wk2.0, wk2.1, "", Some(4.5))
            .await
            .expect("5-day range booked as 4.5");
    assert_eq!(four_half, 4.5);

    // ── the same days can't be booked twice ──
    assert!(
        err_text(
            leave_service::submit_request(
                &pool,
                veteran_m,
                pat.id,
                d(2031, 3, 12),
                d(2031, 3, 12),
                "",
                None
            )
            .await
        )
        .contains("already have"),
        "overlap with the pending 4.5-day request"
    );

    // ── an HR allocation overrides eligibility; "adjust" starts from the ELIGIBLE allotment (0) ──
    leave::upsert_allocation(&pool, newbie_m, mat.id, 2031, 3.0)
        .await
        .unwrap();
    let b = bal(&pool, newbie_m, mat.id, mon).await;
    assert!(b.is_override && b.allotted_days == 3.0);
    leave_service::submit_request(&pool, newbie_m, mat.id, mon, mon, "", None)
        .await
        .expect("granted by HR despite the rule");
    let adjusted = leave::adjust_allocation(&pool, veteran_m, mat.id, 2031, 1.0)
        .await
        .unwrap();
    assert_eq!(
        adjusted,
        Some(1.0),
        "+1 on a man's maternity is 1 day, not 181"
    );

    // ── approval: two pending 6-day requests (10 allotted) approved AT THE SAME TIME → exactly one wins ──
    let dad = person(&pool, &mut made, "dad", Some("Male"), Some(d(2029, 1, 1))).await;
    let hr = person(&pool, &mut made, "hr", None, None).await;
    let (r1, _) = leave_service::submit_request(
        &pool,
        dad,
        pat.id,
        d(2031, 4, 7),
        d(2031, 4, 14),
        "",
        Some(6.0),
    )
    .await
    .unwrap();
    let (r2, _) = leave_service::submit_request(
        &pool,
        dad,
        pat.id,
        d(2031, 5, 5),
        d(2031, 5, 12),
        "",
        Some(6.0),
    )
    .await
    .unwrap();
    let (a1, a2) = tokio::join!(
        leave_service::approve(&pool, r1, hr),
        leave_service::approve(&pool, r2, hr)
    );
    let wins = [
        a1.as_ref().ok() == Some(&true),
        a2.as_ref().ok() == Some(&true),
    ];
    assert_eq!(
        wins.iter().filter(|w| **w).count(),
        1,
        "one approval, one refusal: {a1:?} / {a2:?}"
    );
    assert!(
        bal(&pool, dad, pat.id, mon).await.used_days <= 10.0,
        "never overdrawn"
    );

    // ── "leaves left" excludes paternity/maternity ──
    let left = leave::remaining_paid_by_user(&pool, 2031, None)
        .await
        .unwrap();
    let expected: f64 = leave::balances_as_of(&pool, veteran_f, 2031, mon)
        .await
        .unwrap()
        .iter()
        .filter(|b| b.paid && !b.special)
        .map(|b| b.remaining_days)
        .sum();
    assert!(
        (left.get(&veteran_f).copied().unwrap_or(0.0) - expected).abs() < 1e-9,
        "leaves-left total = regular paid leave only"
    );

    cleanup(&pool, &made, &types).await;
}

#[tokio::test]
async fn type_rules_survive_a_days_only_update() {
    let Some(pool) = real_pool().await else {
        return;
    };
    let tag = Uuid::new_v4().simple().to_string();
    let t = leave::create_type(
        &pool,
        &format!("keep-{tag}"),
        true,
        10.0,
        10.0,
        10.0,
        &leave::TypeRules {
            eligible_gender: Some("male".into()),
            min_tenure_months: 12,
            day_basis: "working".into(),
        },
    )
    .await
    .unwrap();
    // An older client edits only the days: the rule must stay.
    let u = leave::update_type(&pool, t.id, true, 12.0, 12.0, 12.0, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(u.default_days, 12.0);
    assert_eq!(u.eligible_gender.as_deref(), Some("male"));
    assert_eq!(u.min_tenure_months, 12);
    // Explicitly clearing it works.
    let u = leave::update_type(
        &pool,
        t.id,
        true,
        12.0,
        12.0,
        12.0,
        Some(&leave::TypeRules::default()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(u.eligible_gender, None);
    assert_eq!(u.min_tenure_months, 0);
    cleanup(&pool, &[], &[t.id]).await;
}

#[tokio::test]
async fn seeded_paternity_and_maternity_types_exist() {
    let Some(pool) = real_pool().await else {
        return;
    };
    // Matched case-insensitively: a pre-existing "maternity leave" is brought to policy, not duplicated.
    let types = leave::list_types(&pool).await.unwrap();
    let find = |word: &str| -> Vec<&leave::LeaveType> {
        types
            .iter()
            .filter(|t| t.name.to_lowercase().contains(word) && t.name.contains(' '))
            .collect()
    };
    let pat = find("paternity");
    assert_eq!(pat.len(), 1, "exactly one paternity type");
    assert_eq!(
        (
            pat[0].default_days,
            pat[0].eligible_gender.as_deref(),
            pat[0].min_tenure_months,
            pat[0].day_basis.as_str()
        ),
        (10.0, Some("male"), 12, "working")
    );
    let mat = find("maternity");
    assert_eq!(mat.len(), 1, "exactly one maternity type");
    assert_eq!(
        (
            mat[0].default_days,
            mat[0].eligible_gender.as_deref(),
            mat[0].min_tenure_months,
            mat[0].day_basis.as_str()
        ),
        (180.0, Some("female"), 12, "calendar")
    );
}
