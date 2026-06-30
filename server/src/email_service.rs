//! Outbound email (approval notifications + new-employee credentials).
//!
//! Uses SMTP when configured (`SMTP_HOST` etc.); otherwise logs the message so
//! flows work end-to-end in development without a mail server. Port 465 uses
//! implicit TLS (SMTPS); 587/2525 use STARTTLS.

use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

/// Whether a message was actually sent, or only logged (no SMTP configured).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    Sent,
    Logged,
}

/// Send `body` to `to`, or log it when SMTP isn't configured. `log_detail` is
/// extra text shown ONLY in log-mode (e.g. action links) — never pass secrets
/// (passwords) here, so they don't end up in server logs.
async fn dispatch(
    to: &str,
    subject: &str,
    body: String,
    log_detail: Option<&str>,
) -> anyhow::Result<Delivery> {
    let host = std::env::var("SMTP_HOST").unwrap_or_default();
    if host.is_empty() {
        tracing::info!(
            "[email:log-mode] to={} | {}{}",
            to,
            subject,
            log_detail.map(|d| format!("\n{d}")).unwrap_or_default()
        );
        return Ok(Delivery::Logged);
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
        .to(to.parse()?)
        .subject(subject)
        .header(ContentType::TEXT_PLAIN)
        .body(body)?;

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
    Ok(Delivery::Sent)
}

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
    let links = format!("  APPROVE: {}\n  REJECT:  {}", e.approve_url, e.reject_url);
    dispatch(e.owner_email, &subject, body, Some(&links)).await?;
    Ok(())
}

pub struct CredentialsEmail<'a> {
    pub to: &'a str,
    pub name: &'a str,
    /// The address the employee signs in with (same as `to`).
    pub login_email: &'a str,
    pub password: &'a str,
    /// Optional desktop-app download link (`APP_DOWNLOAD_URL`).
    pub download_url: Option<&'a str>,
}

/// Email a newly-created employee their sign-in credentials. Returns whether the
/// message was actually sent (vs. logged because SMTP is unconfigured) so the
/// caller can decide whether HR still needs to share the password manually.
pub async fn send_credentials(e: CredentialsEmail<'_>) -> anyhow::Result<Delivery> {
    let subject = "Your TimeTracker account";
    let download = e
        .download_url
        .map(|u| format!("Download the TimeTracker desktop app: {u}\n\n"))
        .unwrap_or_default();
    let body = format!(
        "Hi {name},\n\nAn account has been created for you on TimeTracker.\n\n\
         Email:    {email}\nPassword: {pass}\n\n\
         Sign in to the TimeTracker desktop app using the credentials above.\n\n\
         {download}\
         For your security, please change your password after your first sign-in \
         and never share it.\n\n(TimeTracker)\n",
        name = e.name,
        email = e.login_email,
        pass = e.password,
        download = download,
    );
    // Password must NOT be logged — pass no log_detail.
    dispatch(e.to, subject, body, None).await
}
