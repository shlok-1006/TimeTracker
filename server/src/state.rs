//! Shared application state, injected into every handler via Axum's `State`.
//!
//! Holds the PostgreSQL pool and the JWT keys. Cheap to clone (both are `Arc`
//! internally / wrapped in `Arc`), so we pass it by value through the router.

use std::sync::Arc;

use sqlx::PgPool;

use crate::gemini_provider::GeminiProvider;
use crate::jwt::JwtKeys;
use crate::linear_service::LinearService;
use crate::storage::StorageClient;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub jwt: Arc<JwtKeys>,
    pub storage: Arc<StorageClient>,
    pub linear: Arc<LinearService>,
    /// Vision AI provider (Gemini 2.5 Flash) for screenshot analysis (STEP 10).
    pub gemini: Arc<GeminiProvider>,
    /// Refresh-token lifetime in seconds.
    pub refresh_ttl_seconds: i64,
}

impl AppState {
    pub fn new(
        db: PgPool,
        jwt: JwtKeys,
        storage: StorageClient,
        linear: LinearService,
        gemini: GeminiProvider,
        refresh_ttl_seconds: i64,
    ) -> Self {
        Self {
            db,
            jwt: Arc::new(jwt),
            storage: Arc::new(storage),
            linear: Arc::new(linear),
            gemini: Arc::new(gemini),
            refresh_ttl_seconds,
        }
    }
}
