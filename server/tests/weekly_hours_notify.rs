//! Weekly Report data layer — the claims the "Send Email" flow depends on (adversarial-review fixes).
//!
//!   * "Sent" is the EMPLOYEE stamp (`employee_notified_at`), not the legacy HR/PM-digest `notified_at`,
//!     so rows the old batch had digested still offer "Send Email".
//!   * The employee slot is claimed atomically — a second claim loses (no double email) — and a failed
//!     send can release it for a retry.
//!   * A PM sees people they manage directly (`user_managers`) AND members of teams they're assigned to
//!     (`team_pms` + `user_teams`); an unrelated PM sees nobody.
//!   * Deactivated users can't be emailed; `week_has_rows` separates "computed" from "never ran".
//!
//! Skips when DATABASE_URL is unset. Uses a 2020 week and fresh users so it never collides.

use chrono::NaiveDate;
use server::db::weekly_hours;
use uuid::Uuid;

async fn real_pool() -> Option<sqlx::PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .ok()
}

async fn seed_user(pool: &sqlx::PgPool, role: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, name, email, password_hash, role) VALUES ($1,$2,$3,'x',$4::user_role)",
    )
    .bind(id)
    .bind(format!("WH {role}"))
    .bind(format!("wh-{id}@test.local"))
    .bind(role)
    .execute(pool)
    .await
    .expect("seed user");
    id
}

async fn seed_report(
    pool: &sqlx::PgPool,
    user: Uuid,
    week: NaiveDate,
    legacy_digest_sent: bool,
) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO weekly_hours_reports
           (id, user_id, week_start, week_end, working_days, required_seconds, worked_seconds,
            shortfall_seconds, compliant, notified_at)
         VALUES ($1,$2,$3,$3::date + 6,5,144000,100000,44000,false, CASE WHEN $4 THEN now() END)",
    )
    .bind(id)
    .bind(user)
    .bind(week)
    .bind(legacy_digest_sent)
    .execute(pool)
    .await
    .expect("seed report");
    id
}

#[tokio::test]
async fn weekly_report_send_flow_is_correct_and_scoped() {
    let Some(pool) = real_pool().await else {
        eprintln!("DATABASE_URL unset — skipping weekly-hours notify test");
        return;
    };
    let week = NaiveDate::from_ymd_opt(2020, 3, 2).unwrap(); // a Monday

    let emp = seed_user(&pool, "employee").await;
    let team_pm = seed_user(&pool, "project_manager").await;
    let direct_pm = seed_user(&pool, "project_manager").await;
    let stranger_pm = seed_user(&pool, "project_manager").await;
    let team = Uuid::new_v4();
    sqlx::query("INSERT INTO teams (id, name) VALUES ($1,$2)")
        .bind(team)
        .bind(format!("WH team {team}"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO user_teams (user_id, team_id) VALUES ($1,$2)")
        .bind(emp)
        .bind(team)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO team_pms (team_id, pm_user_id) VALUES ($1,$2)")
        .bind(team)
        .bind(team_pm)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO user_managers (user_id, manager_id) VALUES ($1,$2)")
        .bind(emp)
        .bind(direct_pm)
        .execute(&pool)
        .await
        .unwrap();

    // The OLD batch digested this row (legacy notified_at set) — the employee was never emailed.
    let report = seed_report(&pool, emp, week, true).await;
    assert!(
        weekly_hours::week_has_rows(&pool, week).await.unwrap(),
        "week is computed"
    );

    // 1) "Sent" must reflect the EMPLOYEE stamp — so a digested row still offers Send Email.
    let hr_view = weekly_hours::list_shortfalls(&pool, week, None)
        .await
        .unwrap();
    let row = hr_view
        .iter()
        .find(|r| r.user_id == emp)
        .expect("HR sees the row");
    assert!(
        row.notified_at.is_none(),
        "legacy digest stamp must NOT read as 'employee emailed'"
    );

    // 2) Scope: direct report + team assignment both see it; an unrelated PM does not.
    let seen = |rows: &Vec<weekly_hours::ShortfallRow>| rows.iter().any(|r| r.user_id == emp);
    assert!(
        seen(
            &weekly_hours::list_shortfalls(&pool, week, Some(direct_pm))
                .await
                .unwrap()
        ),
        "direct manager sees them"
    );
    assert!(
        seen(
            &weekly_hours::list_shortfalls(&pool, week, Some(team_pm))
                .await
                .unwrap()
        ),
        "team-assigned PM sees them"
    );
    assert!(
        !seen(
            &weekly_hours::list_shortfalls(&pool, week, Some(stranger_pm))
                .await
                .unwrap()
        ),
        "unrelated PM sees nobody"
    );
    assert!(
        weekly_hours::find_shortfall(&pool, emp, week, Some(stranger_pm))
            .await
            .unwrap()
            .is_none(),
        "unrelated PM can't email them"
    );

    // 3) Atomic claim: first wins, second loses (no double email); release re-opens it for a retry.
    let first = weekly_hours::claim_employee_notify(&pool, report)
        .await
        .unwrap();
    let second = weekly_hours::claim_employee_notify(&pool, report)
        .await
        .unwrap();
    assert!(first.is_some(), "first claim wins");
    assert!(second.is_none(), "second claim must lose — no double send");
    weekly_hours::release_employee_notify(&pool, report)
        .await
        .unwrap();
    assert!(
        weekly_hours::claim_employee_notify(&pool, report)
            .await
            .unwrap()
            .is_some(),
        "released slot can be re-claimed"
    );
    let after = weekly_hours::find_shortfall(&pool, emp, week, None)
        .await
        .unwrap()
        .unwrap();
    assert!(after.notified_at.is_some(), "now reads as Sent");

    // 4) Deactivated users can't be emailed.
    sqlx::query("UPDATE users SET deactivated_at = now() WHERE id = $1")
        .bind(emp)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        weekly_hours::find_shortfall(&pool, emp, week, None)
            .await
            .unwrap()
            .is_none(),
        "deactivated → not emailable"
    );

    // cleanup
    sqlx::query("DELETE FROM teams WHERE id = $1")
        .bind(team)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM users WHERE id = ANY($1)")
        .bind(vec![emp, team_pm, direct_pm, stranger_pm])
        .execute(&pool)
        .await
        .ok();
    assert!(
        !weekly_hours::week_has_rows(&pool, week).await.unwrap(),
        "no rows → not computed"
    );
}
