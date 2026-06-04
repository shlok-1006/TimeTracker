//! Shared application state, injected into every handler via Axum's `State`.
//!
//! Holds the PostgreSQL pool and the JWT keys. Cheap to clone (both are `Arc`
//! internally / wrapped in `Arc`), so we pass it by value through the router.

use std::sync::Arc;

use sqlx::PgPool;

use crate::jwt::JwtKeys;
use crate::storage::StorageClient;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub jwt: Arc<JwtKeys>,
    pub storage: Arc<StorageClient>,
}

impl AppState {
    pub fn new(db: PgPool, jwt: JwtKeys, storage: StorageClient) -> Self {
        Self {
            db,
            jwt: Arc::new(jwt),
            storage: Arc::new(storage),
        }
    }
}
