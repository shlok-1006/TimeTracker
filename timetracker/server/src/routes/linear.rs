//! Linear integration routes (protected). The Linear token never leaves the
//! server — clients only ever see ticket data.

use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::db::{ticket_requests, users};
use crate::email_service::{self, ApprovalEmail};
use crate::error::AppError;
use crate::middleware::AuthUser;
use crate::state::AppState;

fn public_base() -> String {
    std::env::var("PUBLIC_BASE_URL").unwrap_or_else(|_| "http://localhost:8090".to_string())
}

/// `POST /me/linear/link` — link the caller's account to Linear by email match.
async fn link(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Value>, AppError> {
    let me = users::find_by_id(&state.db, user.id)
        .await?
        .ok_or(AppError::Unauthorized)?;
    let linear_user_id = state
        .linear
        .link_user_to_linear(&state.db, user.id, &me.email)
        .await?;
    Ok(Json(json!({ "linked": true, "linear_user_id": linear_user_id })))
}

/// `GET /me/tickets` — the caller's assigned Linear tickets (cached hourly).
async fn my_tickets(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Value>, AppError> {
    let tickets = state.linear.fetch_assigned_tickets(&state.db, user.id).await?;
    Ok(Json(json!({ "tickets": tickets })))
}

/// `GET /me/tickets/:id/context` — full context for one ticket.
async fn ticket_context(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    match state.linear.get_ticket_context(&id).await? {
        Some(t) => Ok(Json(json!(t))),
        None => Err(AppError::NotFound),
    }
}

#[derive(Deserialize)]
struct TicketRequestBody {
    ticket: String,
}

/// `POST /me/tickets/request` — request access to a ticket by id/identifier.
/// Resolves the ticket's parent owner and emails them an approve/reject link.
async fn request_ticket(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<TicketRequestBody>,
) -> Result<Json<Value>, AppError> {
    let me = users::find_by_id(&state.db, user.id)
        .await?
        .ok_or(AppError::Unauthorized)?;

    let owned = state
        .linear
        .fetch_for_request(body.ticket.trim())
        .await?
        .ok_or_else(|| AppError::BadRequest("ticket not found in Linear".into()))?;

    // Unguessable token used in the emailed approve/reject links.
    let token = format!(
        "{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    );

    ticket_requests::create(
        &state.db,
        user.id,
        &owned.ticket.id,
        Some(&owned.ticket.title),
        owned.owner_email.as_deref(),
        &token,
    )
    .await?;

    let base = public_base();
    let approve_url = format!("{base}/tickets/requests/{token}/approve");
    let reject_url = format!("{base}/tickets/requests/{token}/reject");

    let mut emailed = false;
    if let Some(owner_email) = owned.owner_email.as_deref() {
        match email_service::send_approval_request(ApprovalEmail {
            owner_email,
            owner_name: owned.owner_name.as_deref(),
            employee_name: &me.name,
            ticket_id: &owned.ticket.id,
            ticket_title: &owned.ticket.title,
            approve_url: &approve_url,
            reject_url: &reject_url,
        })
        .await
        {
            Ok(()) => emailed = true,
            Err(e) => tracing::warn!("failed to send approval email: {e}"),
        }
    }

    Ok(Json(json!({
        "status": "pending",
        "ticket": owned.ticket,
        "owner_email": owned.owner_email,
        "emailed": emailed,
    })))
}

/// `GET /me/tickets/requests` — the caller's manual ticket requests.
async fn my_requests(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Value>, AppError> {
    let requests = ticket_requests::list_for_user(&state.db, user.id).await?;
    Ok(Json(json!({ "requests": requests })))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/me/linear/link", post(link))
        .route("/me/tickets", get(my_tickets))
        .route("/me/tickets/:id/context", get(ticket_context))
        .route("/me/tickets/request", post(request_ticket))
        .route("/me/tickets/requests", get(my_requests))
}
