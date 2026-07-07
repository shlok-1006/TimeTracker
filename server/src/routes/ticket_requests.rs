//! Public approve/reject endpoints for ticket access requests. Secured by the
//! one-time `decision_token` embedded in the emailed link (no login required —
//! the ticket owner may be external to the app).

use axum::{
    extract::{Path, State},
    response::Html,
    routing::get,
    Router,
};

use crate::db::ticket_requests as repo;
use crate::error::AppError;
use crate::state::AppState;

/// Minimal HTML-entity escaping for text interpolated into the page (SEC-19).
/// The ticket id in `message` is client-supplied, so it must be escaped.
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

fn page(title: &str, message: &str) -> Html<String> {
    let title = esc(title);
    let message = esc(message);
    Html(format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title>\
         <style>body{{font-family:system-ui;display:flex;height:100vh;align-items:center;\
         justify-content:center;background:#f8fafc;color:#0f172a}}\
         .card{{background:#fff;border:1px solid #e2e8f0;border-radius:12px;padding:32px;max-width:420px;text-align:center}}\
         h1{{font-size:20px;margin:0 0 8px}}p{{color:#475569;margin:0}}</style></head>\
         <body><div class=\"card\"><h1>{title}</h1><p>{message}</p></div></body></html>"
    ))
}

async fn decide(
    state: AppState,
    token: String,
    status: &str,
    verb: &str,
) -> Result<Html<String>, AppError> {
    match repo::decide(&state.db, &token, status).await? {
        Some(ticket) => Ok(page(
            &format!("Request {verb}"),
            &format!("You have {verb} access to ticket {ticket}."),
        )),
        None => Ok(page(
            "Link no longer valid",
            "This request was already decided, or the link is invalid.",
        )),
    }
}

/// RA-05: a confirmation page for GET, so a link prefetch (mail-client preview
/// bots often auto-fetch URLs) can't trigger the decision. The actual state
/// change happens only on the POST from this form.
fn confirm_page(token: &str, verb: &str) -> Html<String> {
    // token is a 64-hex string; esc() it defensively for the attribute context.
    let action = format!("/tickets/requests/{}/{}", esc(token), verb);
    let label = if verb == "approve" { "Approve" } else { "Reject" };
    Html(format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Confirm {label}</title>\
         <style>body{{font-family:system-ui;display:flex;height:100vh;align-items:center;\
         justify-content:center;background:#f8fafc;color:#0f172a}}\
         .card{{background:#fff;border:1px solid #e2e8f0;border-radius:12px;padding:32px;max-width:420px;text-align:center}}\
         h1{{font-size:20px;margin:0 0 8px}}p{{color:#475569;margin:0 0 20px}}\
         button{{font:inherit;padding:10px 20px;border:0;border-radius:8px;background:#4f46e5;color:#fff;cursor:pointer}}</style></head>\
         <body><div class=\"card\"><h1>{label} ticket access?</h1>\
         <p>Click the button to confirm — this link won't act on its own.</p>\
         <form method=\"post\" action=\"{action}\"><button type=\"submit\">{label} access</button></form>\
         </div></body></html>"
    ))
}

async fn approve_confirm(Path(token): Path<String>) -> Html<String> {
    confirm_page(&token, "approve")
}

async fn reject_confirm(Path(token): Path<String>) -> Html<String> {
    confirm_page(&token, "reject")
}

async fn approve(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> Result<Html<String>, AppError> {
    decide(state, token, "approved", "approved").await
}

async fn reject(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> Result<Html<String>, AppError> {
    decide(state, token, "rejected", "rejected").await
}

pub fn router() -> Router<AppState> {
    Router::new()
        // GET shows a confirmation form; POST performs the decision (RA-05).
        .route(
            "/tickets/requests/:token/approve",
            get(approve_confirm).post(approve),
        )
        .route(
            "/tickets/requests/:token/reject",
            get(reject_confirm).post(reject),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_html_metacharacters() {
        let out = esc("<script>alert('x')</script> & \"q\"");
        assert!(!out.contains('<') && !out.contains('>'));
        assert!(out.contains("&lt;script&gt;"));
        assert!(out.contains("&amp;") && out.contains("&quot;") && out.contains("&#x27;"));
    }
}
