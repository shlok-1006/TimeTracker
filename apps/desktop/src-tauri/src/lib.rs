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
mod reminder;
mod reports;
mod screenshot;
mod sync_worker;
mod timer;
mod updates;

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

/// Send `tracing` output somewhere a human can read it.
///
/// Without this the app calls `tracing::warn!` in a dozen places and every one
/// of them is discarded — there is no subscriber, so a background worker can
/// fail forever in total silence. That is precisely how reminders were able to
/// stop working for a whole platform without leaving a trace. Logs go to a file
/// next to the database (the release build is a windowed app with no console),
/// and `RUST_LOG` still works for anyone debugging.
fn init_logging(data_dir: &std::path::Path) {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("timetracker_desktop_lib=info,warn"));

    // Best-effort: a logging failure must never stop the app from starting.
    let file_layer = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(data_dir.join("timetracker.log"))
        .ok()
        .map(|f| {
            tracing_subscriber::fmt::layer()
                .with_writer(std::sync::Mutex::new(f))
                .with_ansi(false)
        });

    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .with(file_layer)
        .try_init();
}

/// Open the local DB, migrate, manage state, and start the background workers.
async fn init(handle: tauri::AppHandle) -> anyhow::Result<()> {
    let data_dir = {
        let dir = handle
            .path()
            .app_data_dir()
            .context("resolve app data dir")?;
        // A debug build MUST NOT share state with the installed app. Tauri derives
        // this path from the bundle identifier, so `tauri dev` and the shipped
        // release land in the same folder and open the same SQLite file — a
        // developer running the app locally would be reading and migrating the
        // real database of whoever is using TimeTracker on that machine. Give
        // debug builds their own sibling directory instead.
        if cfg!(debug_assertions) {
            let mut name = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "com.timetracker.desktop".to_string());
            name.push_str(".dev");
            dir.with_file_name(name)
        } else {
            dir
        }
    };
    // The data dir may not exist yet on a first run; the DB would create it, but
    // logging wants it first so startup problems are captured too.
    let _ = std::fs::create_dir_all(&data_dir);
    init_logging(&data_dir);
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "TimeTracker starting");
    let db_path = data_dir.join("timetracker.db");

    let pool = db::connect(&db_path).await.context("open local database")?;
    db::migrate(&pool).await.context("run local migrations")?;

    // One-time repair for machines upgrading from an earlier unsigned build:
    // clear a stale Screen Recording grant that no longer matches this signed
    // build so the user's fresh grant takes effect (see the function's docs).
    screenshot::repair_stale_grant_if_needed(&data_dir);

    // Idle detection (configurable threshold) + background workers.
    // Idle is now read from the OS on demand (no sampler thread).
    let idle = idle::IdleHandle::from_env();

    let state = timer::DesktopState::new(pool, idle);
    handle.manage(state.clone());
    tauri::async_runtime::spawn(timer::run_recorder(state.clone()));
    tauri::async_runtime::spawn(sync_worker::run(state.clone()));
    tauri::async_runtime::spawn(presence::run(state.clone()));
    tauri::async_runtime::spawn(presence::run_break_reminders(handle.clone(), state.clone()));
    tauri::async_runtime::spawn(presence::run_not_tracking_reminders(
        handle.clone(),
        state.clone(),
    ));
    tauri::async_runtime::spawn(activity_tracker::run(state.clone()));
    tauri::async_runtime::spawn(screenshot::run(state));
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Reminders are raised as a persistent window (see `reminder`), not as
        // OS notifications — those auto-dismiss and cannot report delivery.
        // Launch at system login so tracking resumes when the machine starts.
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
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
            // Register the app to launch at login (idempotent). Best-effort — a
            // failure here shouldn't block startup.
            {
                use tauri_plugin_autostart::ManagerExt;
                if let Err(e) = app.autolaunch().enable() {
                    tracing::warn!("could not enable launch-at-login: {e}");
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            auth::login,
            auth::change_password,
            auth::restore_session,
            auth::session_alive,
            auth::logout,
            timer::start_tracking,
            timer::stop_tracking,
            timer::is_tracking,
            timer::get_total_seconds,
            presence::set_break,
            presence::is_on_break,
            presence::mute_break_reminders,
            presence::break_reminders_muted,
            reminder::current_reminder,
            reminder::dismiss_reminder,
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
            screenshot::request_capture_permission,
            screenshot::relaunch_app,
            updates::check_for_update,
            updates::open_downloads_page
        ])
        .run(tauri::generate_context!())
        .expect("error while running TimeTracker desktop application");
}
