//! Ticket access requests repository (Rule 7).

use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

/// SHA-256 hex of a decision token. Only the hash is stored — the plaintext
/// token lives only in the emailed link (SEC-28), mirroring refresh tokens.
fn hash_token(token: &str) -> String {
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    hex::encode(h.finalize())
}

#[derive(Debug, Serialize)]
pub struct TicketRequest {
    pub id: Uuid,
    pub ticket_id: String,
    pub ticket_title: Option<String>,
    pub owner_email: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
}

/// Create a pending request. Returns the new id.
pub async fn create(
    pool: &PgPool,
    user_id: Uuid,
    ticket_id: &str,
    ticket_title: Option<&str>,
    owner_email: Option<&str>,
    decision_token: &str,
) -> Result<Uuid, AppError> {
    let row = sqlx::query!(
        r#"
        INSERT INTO ticket_requests (user_id, ticket_id, ticket_title, owner_email, decision_token)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id
        "#,
        user_id,
        ticket_id,
        ticket_title,
        owner_email,
        hash_token(decision_token)
    )
    .fetch_one(pool)
    .await?;
    Ok(row.id)
}

/// A user's requests, newest first.
pub async fn list_for_user(pool: &PgPool, user_id: Uuid) -> Result<Vec<TicketRequest>, AppError> {
    let rows = sqlx::query!(
        r#"
        SELECT id, ticket_id, ticket_title, owner_email, status, created_at, decided_at
        FROM ticket_requests WHERE user_id = $1 ORDER BY created_at DESC
        "#,
        user_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| TicketRequest {
            id: r.id,
            ticket_id: r.ticket_id,
            ticket_title: r.ticket_title,
            owner_email: r.owner_email,
            status: r.status,
            created_at: r.created_at,
            decided_at: r.decided_at,
        })
        .collect())
}

/// Whether `user_id` has an **approved** access request for `ticket_id`
/// (authorizes reading that ticket's full context — SEC-12).
pub async fn has_approved(
    pool: &PgPool,
    user_id: Uuid,
    ticket_id: &str,
) -> Result<bool, AppError> {
    let row = sqlx::query!(
        r#"SELECT EXISTS(
              SELECT 1 FROM ticket_requests
              WHERE user_id = $1 AND ticket_id = $2 AND status = 'approved'
           ) AS "approved!""#,
        user_id,
        ticket_id
    )
    .fetch_one(pool)
    .await?;
    Ok(row.approved)
}

/// Apply a decision (approved|rejected) to a pending request by token.
/// Returns the ticket id if a pending request was updated.
pub async fn decide(
    pool: &PgPool,
    token: &str,
    status: &str,
) -> Result<Option<String>, AppError> {
    // Look up by the token hash (SEC-28) and enforce a 7-day validity window.
    let row = sqlx::query!(
        r#"
        UPDATE ticket_requests SET status = $2, decided_at = now()
        WHERE decision_token = $1 AND status = 'pending'
          AND created_at > now() - interval '7 days'
        RETURNING ticket_id
        "#,
        hash_token(token),
        status
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.ticket_id))
}
