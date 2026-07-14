//! Desktop application core.
//!
//! STEP 2 adds the time-recording engine:
//!   * `db`                  — local SQLite (source of truth, Rule 1)
//!   * `interval_repository` — immutable intervals + sync queue
//!   * `timer`               — start/stop tracking (monotonic clock)
//!   * `sync_worker`         — background push to the API (Rule 4)

mod activity_tracker;
mod auth;
mod client;
mod db;
mod http;
mod idle;
mod interval_repository;
mod presence;
mod reports;
mod screenshot;
mod sync_worker;
mod timer;

use anyhow::Context;
use serde::Serialize;
use tauri::Manager;

#[derive(Serialize)]
pub struct AppInfo {
    pub name: &'static str,
    pub version: &'static str,
}

#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo {
        name: "TimeTracker",
        version: env!("CARGO_PKG_VERSION"),
    }
}

/// Open the local DB, migrate, manage state, and start the background workers.
async fn init(handle: tauri::AppHandle) -> anyhow::Result<()> {
    let data_dir = handle
        .path()
        .app_data_dir()
        .context("resolve app data dir")?;
    let db_path = data_dir.join("timetracker.db");

    let pool = db::connect(&db_path).await.context("open local database")?;
    db::migrate(&pool).await.context("run local migrations")?;

    // Idle detection (configurable threshold) + background workers.
    // Idle is now read from the OS on demand (no sampler thread).
    let idle = idle::IdleHandle::from_env();

    let state = timer::DesktopState::new(pool, idle);
    handle.manage(state.clone());
    tauri::async_runtime::spawn(timer::run_recorder(state.clone()));
    tauri::async_runtime::spawn(sync_worker::run(state.clone()));
    tauri::async_runtime::spawn(presence::run(state.clone()));
    tauri::async_runtime::spawn(activity_tracker::run(state.clone()));
    tauri::async_runtime::spawn(screenshot::run(state));
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::block_on(init(handle)).map_err(|err| {
                // The release binary is a windowed app with no console: a panic
                // here is an invisible crash loop. Tell the user what broke.
                let message = format!("{err:#}");
                tracing::error!("startup failed: {message}");
                rfd::MessageDialog::new()
                    .set_level(rfd::MessageLevel::Error)
                    .set_title("TimeTracker failed to start")
                    .set_description(&message)
                    .show();
                err
            })?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            auth::login,
            auth::change_password,
            auth::restore_session,
            auth::logout,
            timer::start_tracking,
            timer::stop_tracking,
            timer::is_tracking,
            timer::get_total_seconds,
            presence::set_break,
            presence::is_on_break,
            presence::set_meeting,
            presence::is_in_meeting,
            presence::current_status,
            presence::heartbeat_now,
            reports::get_hours_summary,
            reports::get_daily_timeline,
            client::me_hours,
            client::me_screenshots,
            client::me_report,
            client::me_teams,
            client::me_team_options,
            client::join_team,
            client::leave_team,
            client::me_tickets,
            client::me_tasks,
            client::me_attendance,
            client::me_leave_types,
            client::me_leave_balance,
            client::me_leave_requests,
            client::request_leave,
            client::cancel_leave,
            client::my_ticket_requests,
            client::request_ticket,
            activity_tracker::activity_today,
            screenshot::check_capture,
            screenshot::request_capture_permission
        ])
        .run(tauri::generate_context!())
        .expect("error while running TimeTracker desktop application");
}
