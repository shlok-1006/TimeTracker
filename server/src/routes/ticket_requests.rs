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

fn page(title: &str, message: &str) -> Html<String> {
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
        .route("/tickets/requests/:token/approve", get(approve))
        .route("/tickets/requests/:token/reject", get(reject))
}
