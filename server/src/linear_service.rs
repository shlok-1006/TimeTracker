//! Linear integration (read-only). Talks to Linear's GraphQL API with a
//! server-held API token (`LINEAR_API_KEY`) that is NEVER exposed to clients.
//!
//! Provides the STEP 8 functions: `link_user_to_linear`, `fetch_assigned_tickets`
//! (hourly cache + stale fallback on rate limit), and `get_ticket_context`.

use std::time::Duration;

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::{linear_repository, users};
use crate::error::AppError;
use crate::ticket_cache::{Ticket, TicketCache};

const LINEAR_GRAPHQL_URL: &str = "https://api.linear.app/graphql";
const CACHE_TTL: Duration = Duration::from_secs(3600); // hourly
const EXCERPT_CHARS: usize = 200;

#[derive(Debug, thiserror::Error)]
enum LinearError {
    #[error("Linear integration is not configured")]
    NotConfigured,
    #[error("Linear rate limit exceeded")]
    RateLimited,
    #[error("Linear request failed: {0}")]
    Http(String),
    #[error("Linear API error: {0}")]
    Api(String),
}

impl From<LinearError> for AppError {
    fn from(e: LinearError) -> Self {
        match e {
            LinearError::NotConfigured => {
                AppError::BadRequest("Linear integration is not configured".into())
            }
            other => AppError::Internal(anyhow::anyhow!(other.to_string())),
        }
    }
}

pub struct LinearService {
    api_key: Option<String>,
    client: reqwest::Client,
    cache: TicketCache,
}

impl LinearService {
    pub fn from_env() -> Self {
        Self {
            api_key: std::env::var("LINEAR_API_KEY").ok().filter(|s| !s.is_empty()),
            client: reqwest::Client::new(),
            cache: TicketCache::new(CACHE_TTL),
        }
    }

    pub fn is_configured(&self) -> bool {
        self.api_key.is_some()
    }

    /// Execute a GraphQL query, returning the `data` object.
    async fn graphql(&self, query: &str, variables: Value) -> Result<Value, LinearError> {
        let key = self.api_key.as_ref().ok_or(LinearError::NotConfigured)?;
        let resp = self
            .client
            .post(LINEAR_GRAPHQL_URL)
            .header("Authorization", key) // Linear personal API key (no "Bearer")
            .json(&json!({ "query": query, "variables": variables }))
            .send()
            .await
            .map_err(|e| LinearError::Http(e.to_string()))?;

        if resp.status().as_u16() == 429 {
            return Err(LinearError::RateLimited);
        }
        if !resp.status().is_success() {
            return Err(LinearError::Api(format!("HTTP {}", resp.status())));
        }
        let body: Value = resp.json().await.map_err(|e| LinearError::Http(e.to_string()))?;
        if let Some(errors) = body.get("errors") {
            return Err(LinearError::Api(errors.to_string()));
        }
        Ok(body.get("data").cloned().unwrap_or(Value::Null))
    }

    /// Resolve a Linear user id by email.
    async fn find_user_by_email(&self, email: &str) -> Result<Option<String>, LinearError> {
        let q = r#"query($email: String!) {
            users(filter: { email: { eq: $email } }) { nodes { id } }
        }"#;
        let data = self.graphql(q, json!({ "email": email })).await?;
        Ok(data
            .pointer("/users/nodes/0/id")
            .and_then(|v| v.as_str())
            .map(String::from))
    }

    /// Fetch OPEN issues assigned to a Linear user. Completed ("done") and
    /// canceled issues are excluded by state type so they don't clutter the
    /// employee dashboard.
    async fn fetch_assigned(&self, linear_user_id: &str) -> Result<Vec<Ticket>, LinearError> {
        let q = r#"query($assignee: ID!) {
            issues(
                filter: {
                    assignee: { id: { eq: $assignee } }
                    state: { type: { nin: ["completed", "canceled"] } }
                }
                first: 50
            ) {
                nodes {
                    id title description
                    state { name }
                    project { name }
                    labels { nodes { name } }
                }
            }
        }"#;
        let data = self.graphql(q, json!({ "assignee": linear_user_id })).await?;
        let nodes = data
            .pointer("/issues/nodes")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(nodes.iter().map(parse_ticket).collect())
    }

    async fn fetch_issue(&self, issue_id: &str) -> Result<Option<Ticket>, LinearError> {
        let q = r#"query($id: String!) {
            issue(id: $id) {
                id title description
                state { name }
                project { name }
                labels { nodes { name } }
            }
        }"#;
        let data = self.graphql(q, json!({ "id": issue_id })).await?;
        Ok(data.get("issue").filter(|v| !v.is_null()).map(parse_ticket))
    }

    // ---- Public orchestration (the STEP 8 functions) ----

    /// Link an internal user to their Linear account by matching email.
    pub async fn link_user_to_linear(
        &self,
        db: &PgPool,
        user_id: Uuid,
        email: &str,
    ) -> Result<String, AppError> {
        match self.find_user_by_email(email).await? {
            Some(linear_id) => {
                linear_repository::upsert(db, user_id, &linear_id).await?;
                Ok(linear_id)
            }
            None => Err(AppError::BadRequest(format!(
                "no Linear user found with email {email}"
            ))),
        }
    }

    /// Best-effort: link a user to their Linear account by matching the email on
    /// their TimeTracker profile. Returns the Linear user id (and stores the
    /// link) if a match is found, else `None`. A transient Linear lookup error
    /// is treated as "not linked this time" so it never breaks the dashboard.
    async fn auto_link(&self, db: &PgPool, user_id: Uuid) -> Result<Option<String>, AppError> {
        let user = match users::find_by_id(db, user_id).await? {
            Some(u) => u,
            None => return Ok(None),
        };
        match self.find_user_by_email(&user.email).await {
            Ok(Some(linear_id)) => {
                linear_repository::upsert(db, user_id, &linear_id).await?;
                tracing::info!(%user_id, email = %user.email, "auto-linked to Linear by email");
                Ok(Some(linear_id))
            }
            Ok(None) => Ok(None),
            Err(e) => {
                tracing::warn!("auto-link email lookup failed ({e}); treating as unlinked");
                Ok(None)
            }
        }
    }

    /// Assigned tickets for a user. Serves the hourly cache; on rate-limit /
    /// failure falls back to the last cached result if available.
    pub async fn fetch_assigned_tickets(
        &self,
        db: &PgPool,
        user_id: Uuid,
    ) -> Result<Vec<Ticket>, AppError> {
        if let Some(cached) = self.cache.fresh(user_id) {
            return Ok(cached);
        }
        // Not linked (or Linear not configured) → no assigned tickets (the UI
        // shows the empty state + manual entry).
        if !self.is_configured() {
            return Ok(vec![]);
        }
        let linear_id = match linear_repository::get_linear_user_id(db, user_id).await? {
            Some(id) => id,
            // Not linked yet → try to auto-link by matching the employee's email
            // to a Linear account. No match → no assigned tickets.
            None => match self.auto_link(db, user_id).await? {
                Some(id) => id,
                None => return Ok(vec![]),
            },
        };

        match self.fetch_assigned(&linear_id).await {
            Ok(tickets) => {
                self.cache.put(user_id, tickets.clone());
                Ok(tickets)
            }
            Err(e) => match self.cache.stale(user_id) {
                Some(stale) => {
                    tracing::warn!("linear fetch failed ({e}); serving stale cache");
                    Ok(stale)
                }
                None => Err(e.into()),
            },
        }
    }

    /// Full context for one ticket (used by later AI features).
    pub async fn get_ticket_context(&self, issue_id: &str) -> Result<Option<Ticket>, AppError> {
        Ok(self.fetch_issue(issue_id).await?)
    }

    /// Resolve a ticket (by UUID or identifier) and its owner for an access
    /// request. The owner is taken from the ticket's **parent** (assignee, else
    /// creator), falling back to the ticket's own assignee/creator.
    pub async fn fetch_for_request(&self, input: &str) -> Result<Option<OwnedTicket>, AppError> {
        if !self.is_configured() {
            return Err(AppError::BadRequest(
                "Linear integration is not configured".into(),
            ));
        }
        let node = self.resolve_issue_node(input).await?;
        Ok(node.map(|n| {
            let (owner_email, owner_name) = owner_of(&n);
            OwnedTicket {
                ticket: parse_ticket(&n),
                owner_email,
                owner_name,
            }
        }))
    }

    /// Look up an issue node by UUID (`issue`) or, failing that, by search.
    async fn resolve_issue_node(&self, input: &str) -> Result<Option<Value>, LinearError> {
        let by_id = format!("query($id: String!) {{ issue(id: $id) {{ {ISSUE_FIELDS} }} }}");
        if let Ok(data) = self.graphql(&by_id, json!({ "id": input })).await {
            if let Some(n) = data.get("issue").filter(|v| !v.is_null()) {
                return Ok(Some(n.clone()));
            }
        }
        let by_search =
            format!("query($q: String!) {{ issueSearch(query: $q, first: 1) {{ nodes {{ {ISSUE_FIELDS} }} }} }}");
        match self.graphql(&by_search, json!({ "q": input })).await {
            Ok(data) => Ok(data.pointer("/issueSearch/nodes/0").filter(|v| !v.is_null()).cloned()),
            Err(_) => Ok(None),
        }
    }
}

/// A ticket plus the owner to ask for approval.
#[derive(Debug)]
pub struct OwnedTicket {
    pub ticket: Ticket,
    pub owner_email: Option<String>,
    pub owner_name: Option<String>,
}

/// GraphQL field selection including parent + owners.
const ISSUE_FIELDS: &str = "id identifier title description \
    state { name } project { name } labels { nodes { name } } \
    parent { assignee { email name } creator { email name } } \
    assignee { email name } creator { email name }";

/// Owner email+name: parent.assignee → parent.creator → assignee → creator.
fn owner_of(node: &Value) -> (Option<String>, Option<String>) {
    for path in [
        "/parent/assignee",
        "/parent/creator",
        "/assignee",
        "/creator",
    ] {
        let email = node
            .pointer(&format!("{path}/email"))
            .and_then(|v| v.as_str());
        if let Some(email) = email {
            let name = node
                .pointer(&format!("{path}/name"))
                .and_then(|v| v.as_str())
                .map(String::from);
            return (Some(email.to_string()), name);
        }
    }
    (None, None)
}

fn str_at(v: &Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or_default().to_string()
}

/// Parse a Linear `issue` GraphQL node into our `Ticket` shape.
fn parse_ticket(node: &Value) -> Ticket {
    let labels = node
        .pointer("/labels/nodes")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|l| l.get("name").and_then(|x| x.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();

    Ticket {
        id: str_at(node, "id"),
        title: str_at(node, "title"),
        state: node
            .pointer("/state/name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        project: node
            .pointer("/project/name")
            .and_then(|v| v.as_str())
            .map(String::from),
        labels,
        description_excerpt: excerpt(node.get("description").and_then(|v| v.as_str())),
    }
}

/// One-line, length-capped excerpt of a (possibly markdown) description.
fn excerpt(desc: Option<&str>) -> String {
    let oneline = desc.unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ");
    if oneline.chars().count() > EXCERPT_CHARS {
        let s: String = oneline.chars().take(EXCERPT_CHARS).collect();
        format!("{s}…")
    } else {
        oneline
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_linear_issue_node() {
        let node = json!({
            "id": "ISS-1",
            "title": "Fix login bug",
            "description": "Users  cannot\nlog in  on Safari.",
            "state": { "name": "In Progress" },
            "project": { "name": "Ruh AI" },
            "labels": { "nodes": [ { "name": "bug" }, { "name": "urgent" } ] }
        });
        let t = parse_ticket(&node);
        assert_eq!(t.id, "ISS-1");
        assert_eq!(t.title, "Fix login bug");
        assert_eq!(t.state, "In Progress");
        assert_eq!(t.project.as_deref(), Some("Ruh AI"));
        assert_eq!(t.labels, vec!["bug", "urgent"]);
        assert_eq!(t.description_excerpt, "Users cannot log in on Safari."); // normalized whitespace
    }

    #[test]
    fn handles_missing_fields() {
        let t = parse_ticket(&json!({ "id": "X", "title": "T" }));
        assert_eq!(t.state, "");
        assert!(t.project.is_none());
        assert!(t.labels.is_empty());
        assert_eq!(t.description_excerpt, "");
    }

    #[test]
    fn owner_prefers_parent_assignee_then_falls_back() {
        let with_parent = json!({
            "parent": { "assignee": { "email": "lead@x.com", "name": "Lead" },
                        "creator":  { "email": "creator@x.com", "name": "Creator" } },
            "assignee": { "email": "emp@x.com", "name": "Emp" }
        });
        assert_eq!(
            owner_of(&with_parent),
            (Some("lead@x.com".into()), Some("Lead".into()))
        );

        // No parent → fall back to the ticket's own assignee.
        let no_parent = json!({ "assignee": { "email": "emp@x.com", "name": "Emp" } });
        assert_eq!(owner_of(&no_parent), (Some("emp@x.com".into()), Some("Emp".into())));

        // Nothing → none.
        assert_eq!(owner_of(&json!({})), (None, None));
    }

    #[test]
    fn excerpt_truncates_long_text() {
        let long = "word ".repeat(100);
        let e = excerpt(Some(&long));
        assert!(e.ends_with('…'));
        assert!(e.chars().count() <= EXCERPT_CHARS + 1);
    }
}
