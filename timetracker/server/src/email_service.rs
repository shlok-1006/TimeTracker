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
    let subject = format!(
        "[TimeTracker] {} requests access to {}",
        e.employee_name, e.ticket_id
    );
    let body = format!(
        "Hi {owner},\n\n{emp} would like to work on ticket {tid} — \"{title}\".\n\n\
         Approve: {approve}\nReject:  {reject}\n\n\
         (TimeTracker)\n",
        owner = e.owner_name.unwrap_or("there"),
        emp = e.employee_name,
        tid = e.ticket_id,
        title = e.ticket_title,
        approve = e.approve_url,
        reject = e.reject_url,
    );

    let host = std::env::var("SMTP_HOST").unwrap_or_default();
    if host.is_empty() {
        // Log-mode: no SMTP configured — surface the email + action links.
        tracing::info!(
            "[email:log-mode] to={} | {}\n  APPROVE: {}\n  REJECT:  {}",
            e.owner_email,
            subject,
            e.approve_url,
            e.reject_url
        );
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

    let mut builder = AsyncSmtpTransport::<Tokio1Executor>::relay(&host)?.port(port);
    if !user.is_empty() {
        builder = builder.credentials(Credentials::new(user, pass));
    }
    builder.build().send(message).await?;
    Ok(())
}
