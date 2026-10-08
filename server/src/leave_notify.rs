//! Leave emails for employees who manage people ("managers by assignment").
//!
//! An `employee` can be assigned as someone's manager in `user_managers` (a team lead who isn't a
//! project manager). They see their people's work and approve their leave under "My Team" in the
//! HRMS. When one of their people requests leave they're told it's waiting for them; when it's
//! approved or rejected (by them, HR or a PM) they get a short notice for planning cover. Project
//! managers aren't emailed here — they never were, and they work from the approval queue.
//!
//! Sent off the request path (`spawn`), so booking or deciding leave never waits on SMTP; a failure
//! is logged and dropped. Without `SMTP_HOST` the email is logged instead (email_service log-mode).

use sqlx::PgPool;
use uuid::Uuid;

use crate::db::{leave, users};
use crate::email_service;
use crate::role::UserRole;
use crate::validate::sanitize_line;

/// What happened to the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaveEvent {
    Requested,
    Approved,
    Rejected,
}

/// Fire-and-forget: email the employee-managers (leads) of the request's owner about `event`.
/// `actor` (whoever approved/rejected) is never emailed about their own decision.
pub fn spawn_notify(pool: PgPool, leave_id: Uuid, event: LeaveEvent, actor: Option<Uuid>) {
    tokio::spawn(async move {
        if let Err(e) = notify(&pool, leave_id, event, actor).await {
            tracing::warn!(%leave_id, ?event, "leave FYI email failed: {e:#}");
        }
    });
}

async fn notify(
    pool: &PgPool,
    leave_id: Uuid,
    event: LeaveEvent,
    actor: Option<Uuid>,
) -> anyhow::Result<()> {
    let Some(req) = leave::request_summary(pool, leave_id).await? else {
        return Ok(());
    };
    let recipients = pick_recipients(users::managers_with_roles(pool, req.user_id).await?, actor);
    if recipients.is_empty() {
        return Ok(());
    }
    let (subject, body) = compose(&req, event);
    email_service::send_plain(&recipients, &subject, &body).await
}

/// Which managers get the email: leads (employee-role managers) only, minus whoever made the decision.
fn pick_recipients(
    managers: Vec<(Uuid, String, String, UserRole)>,
    actor: Option<Uuid>,
) -> Vec<String> {
    managers
        .into_iter()
        .filter(|(id, _, _, role)| *role == UserRole::Employee && Some(*id) != actor)
        .map(|(_, _, email, _)| email)
        .collect()
}

/// Subject and plain-text body. Pure, so the wording is unit-tested.
pub fn compose(req: &leave::RequestSummary, event: LeaveEvent) -> (String, String) {
    let who = sanitize_line(&req.employee_name, 120);
    let kind = sanitize_line(&req.leave_type_name, 80);
    let when = if req.start_date == req.end_date {
        req.start_date.format("%a %-d %b %Y").to_string()
    } else {
        format!(
            "{} – {}",
            req.start_date.format("%a %-d %b"),
            req.end_date.format("%a %-d %b %Y")
        )
    };
    let days = if req.days == 1.0 {
        "1 day".to_string()
    } else {
        format!("{} days", req.days)
    };
    let (what, next) = match event {
        LeaveEvent::Requested => (
            format!("{who} requested leave"),
            "It's waiting for your approval: approve or reject it under My Team in RUH HRMS. HR or their project manager can also decide it.",
        ),
        LeaveEvent::Approved => (
            format!("{who}'s leave was approved"),
            "Plan their cover for those days.",
        ),
        LeaveEvent::Rejected => (
            format!("{who}'s leave request was rejected"),
            "They're expected to work on those days.",
        ),
    };
    let subject = match event {
        LeaveEvent::Requested => format!("Leave to approve: {what} ({when})"),
        _ => format!("FYI: {what} ({when})"),
    };
    let body = format!(
        "{what}.\n\n  Type:  {kind}\n  When:  {when}\n  Length: {days}\n\n{next}\n\n\
         You're getting this because {who} is assigned to you in TimeTracker."
    );
    (subject, body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn req(start: (i32, u32, u32), end: (i32, u32, u32), days: f64) -> leave::RequestSummary {
        leave::RequestSummary {
            user_id: Uuid::nil(),
            employee_name: "Asha\r\nBcc: x@evil".into(),
            leave_type_name: "Casual".into(),
            start_date: NaiveDate::from_ymd_opt(start.0, start.1, start.2).unwrap(),
            end_date: NaiveDate::from_ymd_opt(end.0, end.1, end.2).unwrap(),
            days,
            status: "pending".into(),
        }
    }

    #[test]
    fn a_request_asks_the_lead_to_decide() {
        let (subject, body) = compose(
            &req((2026, 10, 12), (2026, 10, 12), 1.0),
            LeaveEvent::Requested,
        );
        assert_eq!(
            subject,
            "Leave to approve: AshaBcc: x@evil requested leave (Mon 12 Oct 2026)"
        );
        assert!(
            !subject.contains('\n') && !subject.contains('\r'),
            "no header injection"
        );
        assert!(body.contains("Length: 1 day"));
        assert!(body.contains("waiting for your approval") && body.contains("My Team"));
    }

    #[test]
    fn only_leads_are_emailed_and_never_about_their_own_decision() {
        let (lead, other_lead, pm) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let managers = vec![
            (lead, "L".into(), "lead@x".into(), UserRole::Employee),
            (other_lead, "O".into(), "other@x".into(), UserRole::Employee),
            (pm, "P".into(), "pm@x".into(), UserRole::ProjectManager),
        ];
        assert_eq!(
            pick_recipients(managers.clone(), None),
            vec!["lead@x".to_string(), "other@x".to_string()]
        );
        assert_eq!(
            pick_recipients(managers, Some(lead)),
            vec!["other@x".to_string()],
            "the lead who decided isn't told about their own decision"
        );
    }

    #[test]
    fn ranges_and_decisions() {
        let (s, b) = compose(
            &req((2026, 10, 12), (2026, 10, 14), 2.5),
            LeaveEvent::Approved,
        );
        assert_eq!(
            s,
            "FYI: AshaBcc: x@evil's leave was approved (Mon 12 Oct – Wed 14 Oct 2026)"
        );
        assert!(b.contains("Length: 2.5 days") && b.contains("Plan their cover"));
        let (s, _) = compose(
            &req((2026, 10, 12), (2026, 10, 12), 0.5),
            LeaveEvent::Rejected,
        );
        assert!(s.contains("leave request was rejected"));
    }
}
