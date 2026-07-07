//! Outbound email (approval notifications). Uses SMTP when configured
//! (`SMTP_HOST` etc.); otherwise logs the message + action links so the flow
//! works end-to-end in development without a mail server.

use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

pub struct ApprovalEmail<'a> {
    pub owner_email: &'a str,
    pub owner_name: Option<&'a str>,
    pub employee_name: &'a str,
    pub ticket_id: &'a str,
    pub ticket_title: &'a str,
    pub approve_url: &'a str,
    pub reject_url: &'a str,
}

/// Email a ticket's owner asking them to approve/reject an access request.
pub async fn send_approval_request(e: ApprovalEmail<'_>) -> anyhow::Result<()> {
    // SEC-26: strip control characters and cap length on interpolated fields
    // (employee name is internal; ticket id/title/owner come from Linear).
    use crate::validate::sanitize_line;
    let emp = sanitize_line(e.employee_name, 200);
    let tid = sanitize_line(e.ticket_id, 100);
    let title = sanitize_line(e.ticket_title, 300);
    let owner = e.owner_name.map(|o| sanitize_line(o, 200));

    let subject = format!("[TimeTracker] {emp} requests access to {tid}");
    let body = format!(
        "Hi {owner},\n\n{emp} would like to work on ticket {tid} — \"{title}\".\n\n\
         Approve: {approve}\nReject:  {reject}\n\n\
         (TimeTracker)\n",
        owner = owner.as_deref().unwrap_or("there"),
        approve = e.approve_url,
        reject = e.reject_url,
    );

    let host = std::env::var("SMTP_HOST").unwrap_or_default();
    if host.is_empty() {
        // Log-mode: no SMTP configured. The action links embed a one-time
        // decision token, so they are suppressed by default (SEC-29) — set
        // EMAIL_DEBUG_LINKS=true only in local dev to print them.
        if std::env::var("EMAIL_DEBUG_LINKS").map(|v| v == "true").unwrap_or(false) {
            tracing::info!(
                "[email:log-mode] to={} | {}\n  APPROVE: {}\n  REJECT:  {}",
                e.owner_email,
                subject,
                e.approve_url,
                e.reject_url
            );
        } else {
            tracing::info!(
                "[email:log-mode] to={} | {} (approve/reject links suppressed; \
                 set EMAIL_DEBUG_LINKS=true in dev to print them)",
                e.owner_email,
                subject
            );
        }
        return Ok(());
    }

    let from = std::env::var("SMTP_FROM").unwrap_or_else(|_| "timetracker@localhost".to_string());
    let port: u16 = std::env::var("SMTP_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(587);
    let user = std::env::var("SMTP_USER").unwrap_or_default();
    let pass = std::env::var("SMTP_PASS").unwrap_or_default();

    let message = Message::builder()
        .from(from.parse()?)
        .to(e.owner_email.parse()?)
        .subject(subject)
        .header(ContentType::TEXT_PLAIN)
        .body(body)?;

    // Port 465 = implicit TLS (SMTPS); 587/2525 = STARTTLS. Pick the matching
    // transport so the common provider configs work.
    let mut builder = if port == 465 {
        AsyncSmtpTransport::<Tokio1Executor>::relay(&host)?
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&host)?
    }
    .port(port);
    if !user.is_empty() {
        builder = builder.credentials(Credentials::new(user, pass));
    }
    builder.build().send(message).await?;
    Ok(())
}

/// Details for a weekly hours-shortfall warning to HR + the employee's PM.
pub struct HoursShortfallEmail<'a> {
    pub recipients: &'a [String],
    pub employee_name: &'a str,
    pub employee_email: &'a str,
    pub week_start: chrono::NaiveDate,
    pub week_end: chrono::NaiveDate,
    pub working_days: i64,
    pub required_seconds: i64,
    pub worked_seconds: i64,
    pub shortfall_seconds: i64,
}

/// Warn HR and the project manager that an employee missed their weekly hours.
pub async fn send_hours_shortfall(e: HoursShortfallEmail<'_>) -> anyhow::Result<()> {
    // RA-11: strip control chars / cap the interpolated identity fields.
    use crate::validate::sanitize_line;
    let name = sanitize_line(e.employee_name, 200);
    let email = sanitize_line(e.employee_email, 320);
    let subject = format!(
        "[TimeTracker] Weekly hours shortfall — {} ({} to {})",
        name, e.week_start, e.week_end
    );
    let body = format!(
        "Hi,\n\n{name} ({email}) did not complete the expected working hours for the \
         week of {ws} to {we}.\n\n\
         Working days: {wd}\n\
         Expected:     {req}\n\
         Worked:       {wk}\n\
         Shortfall:    {sf}\n\n\
         Please follow up with the employee.\n\n(TimeTracker)\n",
        name = name,
        email = email,
        ws = e.week_start,
        we = e.week_end,
        wd = e.working_days,
        req = fmt_hm(e.required_seconds),
        wk = fmt_hm(e.worked_seconds),
        sf = fmt_hm(e.shortfall_seconds),
    );
    send_plain(e.recipients, &subject, &body).await
}

/// Details for a low daily-score alert to HR.
pub struct LowScoreEmail<'a> {
    pub recipients: &'a [String],
    pub employee_name: &'a str,
    pub employee_email: &'a str,
    pub day: chrono::NaiveDate,
    pub score: f64,
    pub threshold: f64,
    pub total_analyzed: i32,
    pub summary: &'a str,
}

/// Alert HR that an employee's daily alignment score fell below the threshold.
pub async fn send_low_score_alert(e: LowScoreEmail<'_>) -> anyhow::Result<()> {
    // RA-11: sanitize identity fields and the AI-generated summary (model text).
    use crate::validate::sanitize_line;
    let name = sanitize_line(e.employee_name, 200);
    let email = sanitize_line(e.employee_email, 320);
    let summary = sanitize_line(e.summary, 1000);
    let subject = format!(
        "[TimeTracker] Low productivity score — {} ({}: {:.0}%)",
        name, e.day, e.score
    );
    let body = format!(
        "Hi,\n\n{name} ({email}) had a daily alignment score of {score:.0}% on {day}, \
         which is below the {th:.0}% threshold.\n\n\
         Screenshots analysed: {n}\n\
         Summary: {summary}\n\n\
         Please review with the employee.\n\n(TimeTracker)\n",
        name = name,
        email = email,
        score = e.score,
        day = e.day,
        th = e.threshold,
        n = e.total_analyzed,
        summary = summary,
    );
    send_plain(e.recipients, &subject, &body).await
}

/// Send a plaintext message to one or more recipients. Falls back to logging
/// when SMTP isn't configured (dev), mirroring `send_approval_request`.
pub async fn send_plain(recipients: &[String], subject: &str, body: &str) -> anyhow::Result<()> {
    let host = std::env::var("SMTP_HOST").unwrap_or_default();
    if host.is_empty() {
        // Log-mode: no SMTP configured — surface the message so the flow works
        // end-to-end in development.
        tracing::info!("[email:log-mode] to={:?} | {}\n{}", recipients, subject, body);
        return Ok(());
    }
    if recipients.is_empty() {
        return Ok(());
    }

    let from = std::env::var("SMTP_FROM").unwrap_or_else(|_| "timetracker@localhost".to_string());
    let port: u16 = std::env::var("SMTP_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(587);
    let user = std::env::var("SMTP_USER").unwrap_or_default();
    let pass = std::env::var("SMTP_PASS").unwrap_or_default();

    let mut builder = if port == 465 {
        AsyncSmtpTransport::<Tokio1Executor>::relay(&host)?
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&host)?
    }
    .port(port);
    if !user.is_empty() {
        builder = builder.credentials(Credentials::new(user, pass));
    }
    let transport = builder.build();

    for to in recipients {
        let message = Message::builder()
            .from(from.parse()?)
            .to(to.parse()?)
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .body(body.to_string())?;
        transport.send(message).await?;
    }
    Ok(())
}

/// Format a duration in seconds as `"40h 00m"` for human-readable email bodies.
fn fmt_hm(seconds: i64) -> String {
    let s = seconds.max(0);
    format!("{}h {:02}m", s / 3600, (s % 3600) / 60)
}
