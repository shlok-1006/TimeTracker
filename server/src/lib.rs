//! TimeTracker API server library.
//!
//! The binary (`src/main.rs`) is a thin wrapper around this crate so that
//! integration tests can build the real router (Rule 9).

pub mod auth;
pub mod config;
pub mod db;
pub mod email_service;
pub mod error;
pub mod gemini_provider;
pub mod jwt;
pub mod linear_service;
pub mod middleware;
pub mod presence;
pub mod role;
pub mod routes;
pub mod sampler;
pub mod state;
pub mod storage;
pub mod ticket_cache;
pub mod upload_service;
pub mod vision_analyzer;

pub use routes::build as build_router;
pub use state::AppState;
