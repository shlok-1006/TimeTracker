//! TimeTracker API server library.
//!
//! The binary (`src/main.rs`) is a thin wrapper around this crate so that
//! integration tests can build the real router (Rule 9).

pub mod analysis_scheduler;
pub mod analysis_service;
pub mod attendance_scheduler;
pub mod attendance_service;
pub mod auth;
pub mod config;
pub mod db;
pub mod email_service;
pub mod error;
pub mod gemini_provider;
pub mod jwt;
pub mod leave_service;
pub mod linear_service;
pub mod middleware;
pub mod presence;
pub mod report_service;
pub mod role;
pub mod routes;
pub mod sampler;
pub mod state;
pub mod storage;
pub mod summary_generator;
pub mod ticket_cache;
pub mod upload_service;
pub mod vision_analyzer;

pub use routes::build as build_router;
pub use state::AppState;
