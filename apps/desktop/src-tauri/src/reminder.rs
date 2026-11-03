//! Persistent, acknowledge-to-dismiss reminders.
//!
//! Reminders used to go out as native OS notifications. That channel cannot
//! satisfy "stay on screen until the user clicks Ok":
//!
//! * macOS delivers via the deprecated `NSUserNotificationCenter`, whose default
//!   presentation is a *banner* that the OS auto-dismisses after a few seconds.
//!   The `NSUserNotificationAlertStyle` Info.plist key that would make it an
//!   `alert` only seeds the default at first registration — the per-user choice
//!   then lives in `com.apple.ncprefs.plist` and is owned by System Settings, so
//!   shipping the key does nothing for anyone who already runs the app. (It has
//!   also had two never-fixed Apple radars since 10.8.)
//! * `tauri-plugin-notification` spawns the delivery and drops the result, so a
//!   failed notification is indistinguishable from a delivered one.
//! * Its `permission_state()` is hardcoded to `Granted` on desktop, so a user
//!   who turned notifications off (or is in a Focus mode) is undetectable.
//!
//! A break reminder is aimed at someone who is *by definition away from their
//! desk*, so a notification that evaporates after five seconds is the one thing
//! it must not be. This module owns a small always-on-top window instead. It is
//! ours end to end: identical on macOS, Windows and Linux, needs no entitlement
//! and no notification permission, and it stays put until acknowledged.

use serde::Serialize;
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

/// The single reminder window. One label means a second reminder re-uses the
/// window that is already up rather than stacking copies on screen — the loops
/// fire every few minutes and a user who steps away for an hour should come
/// back to one dialog, not twenty.
const WINDOW_LABEL: &str = "reminder";

/// What the reminder window should say. Held here rather than passed in the URL
/// so the window can be reused and re-pointed at a different reminder without a
/// navigation, and so the payload survives the page reloading itself.
#[derive(Clone, Serialize)]
pub struct Reminder {
    /// Discriminates behaviour in the UI (`break` also offers "Don't remind me").
    pub kind: &'static str,
    pub title: String,
    pub body: String,
}

impl Reminder {
    pub fn on_break() -> Self {
        Self {
            kind: "break",
            title: "Still on a break?".into(),
            body: "TimeTracker is paused and isn't recording your time. Resume when you're back."
                .into(),
        }
    }

    pub fn not_tracking() -> Self {
        Self {
            kind: "not_tracking",
            title: "You haven't started the timer".into(),
            body: "Your computer is active but TimeTracker isn't recording. Open the app and click Start tracking."
                .into(),
        }
    }
}

/// What the window should currently display. Written before the window is
/// opened and read back by the page once it loads.
static PENDING: Mutex<Option<Reminder>> = Mutex::new(None);

/// Read by the reminder page on mount to learn what to render.
#[tauri::command]
pub fn current_reminder() -> Option<Reminder> {
    PENDING.lock().ok().and_then(|g| g.clone())
}

/// Is a reminder of this `kind` on screen right now (shown and not yet
/// dismissed)?
pub fn showing(kind: &str) -> bool {
    PENDING
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|r| r.kind == kind))
        .unwrap_or(false)
}

// On macOS the reminder is an NSPanel rather than a plain window: a
// non-activating floating panel is the only kind of window macOS will show in
// another app's full-screen Space. Clicking it doesn't activate TimeTracker or
// pull the user out of what they were in.
#[cfg(target_os = "macos")]
tauri_nspanel::tauri_panel! {
    panel!(ReminderPanel {
        config: {
            can_become_key_window: true,
            is_floating_panel: true
        }
    })
}

/// "Ok" — the user acknowledged, so close the window. Closing is done here
/// rather than from the webview so the window needs no capability of its own.
#[tauri::command]
pub fn dismiss_reminder(app: AppHandle) {
    close(&app, "user clicked Ok");
}

/// Raise a reminder, or bring the existing one back to the front.
///
/// Best-effort by design: a reminder that cannot be shown must never take down
/// the loop that raised it, so every failure is logged and swallowed. Unlike the
/// notification plugin, though, the failure *is* logged — a silent reminder is
/// exactly the bug this module exists to fix.
pub fn show(app: &AppHandle, reminder: Reminder) {
    if let Ok(mut pending) = PENDING.lock() {
        *pending = Some(reminder);
    }
    let app = app.clone();
    // Window creation must happen on the main thread; these callers are all
    // background tasks.
    if let Err(e) = app.clone().run_on_main_thread(move || {
        if let Err(e) = build_or_focus(&app) {
            tracing::warn!("could not show reminder window: {e}");
        }
    }) {
        tracing::warn!("could not reach the main thread to show a reminder: {e}");
    }
}

/// Close the reminder if one is open. Used by "Ok" and by the state changes that
/// make a reminder moot (resuming from a break, starting the timer) so the user
/// never has to dismiss a dialog that has already answered itself.
pub fn close(app: &AppHandle, reason: &'static str) {
    let app = app.clone();
    let _ = app.clone().run_on_main_thread(move || {
        // macOS: the panel is hidden, never closed, and reused by the next
        // reminder. Closing it — even after turning it back into a Tauri
        // window — raised an Objective-C exception that aborted the app the
        // moment the user clicked Ok.
        #[cfg(target_os = "macos")]
        {
            use tauri_nspanel::ManagerExt;
            if let Ok(panel) = app.get_webview_panel(WINDOW_LABEL) {
                tracing::info!(reason, "hiding reminder panel");
                panel.hide();
                if let Ok(mut pending) = PENDING.lock() {
                    *pending = None;
                }
                return;
            }
        }
        if let Some(w) = app.get_webview_window(WINDOW_LABEL) {
            tracing::info!(reason, "closing reminder window");
            let _ = w.close();
        }
        if let Ok(mut pending) = PENDING.lock() {
            *pending = None;
        }
    });
}

/// Put the reminder in the top-right corner rather than the middle of the screen.
///
/// Centring looks right and behaves terribly: the window lands under wherever the
/// pointer already is, so the user's very next click — meant for whatever they
/// were doing — hits the dismiss button and the reminder disappears unread. That
/// was observed repeatedly, with genuine (`isTrusted`) clicks landing on Ok about
/// two seconds after each appearance, at slightly different points each time.
/// An arming delay does not help; the clicks arrive long after any sane delay.
///
/// Every real toast — Windows, macOS, Linux — lives in a corner for this reason.
/// Best-effort: if the monitor can't be resolved the window keeps its default
/// position, which is worse but not broken.
fn park_in_corner(win: &tauri::WebviewWindow) {
    const MARGIN: i32 = 24;
    let Ok(Some(monitor)) = win.current_monitor() else {
        return;
    };
    let Ok(size) = win.outer_size() else { return };
    let scale = monitor.scale_factor();
    let margin = (MARGIN as f64 * scale).round() as i32;
    let area = monitor.size();
    let origin = monitor.position();
    let x = origin.x + area.width as i32 - size.width as i32 - margin;
    let y = origin.y + margin;
    let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
}

fn build_or_focus(app: &AppHandle) -> tauri::Result<()> {
    // Already up: re-assert it. `show` pulls it back into view, and the page
    // polls for the payload so it picks up a different reminder without needing
    // to be rebuilt. Deliberately no `set_focus` — see below.
    if let Some(w) = app.get_webview_window(WINDOW_LABEL) {
        // macOS: the panel kept from an earlier reminder (see `close`). Reload
        // the page so it reads the new payload and re-arms its click guard,
        // then order the panel front — Tauri's `show` would make it key.
        #[cfg(target_os = "macos")]
        {
            let _ = w.reload();
            float_above_other_apps(&w);
            keep_raised(app);
            tracing::info!("reminder panel shown again");
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = w.unminimize();
            w.show()?;
        }
        return Ok(());
    }

    let win = WebviewWindowBuilder::new(app, WINDOW_LABEL, WebviewUrl::App("reminder/".into()))
        .title("TimeTracker")
        .inner_size(460.0, 210.0)
        .resizable(false)
        .minimizable(false)
        .maximizable(false)
        // The point of the exercise: it sits above other windows, and follows
        // the user across spaces/desktops instead of being stranded on the one
        // they left.
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        // Not a task the user switches to — it's a prompt.
        .skip_taskbar(true)
        .decorations(false)
        // Deliberately NOT focused. `always_on_top` is what makes the reminder
        // impossible to miss; stealing the keyboard is a different thing and an
        // actively harmful one. A reminder can arrive while someone is typing —
        // a password, a message — and a focus grab both swallows those
        // keystrokes and lets a stray Enter/Space activate the dismiss button,
        // so the reminder destroys itself before it has been read. Observed
        // exactly that: the window was dismissed 12s after appearing by
        // keystrokes meant for the login form. Visible but not focused means
        // the user finishes their sentence and dismisses it deliberately.
        .focused(false)
        .build()?;

    park_in_corner(&win);
    float_above_other_apps(&win);
    #[cfg(target_os = "macos")]
    keep_raised(app);

    tracing::info!("reminder window shown");
    Ok(())
}

/// macOS: make the reminder cover whatever the user is in, not just the desktop.
///
/// Tauri's `always_on_top` only gets halfway there: from a background app the
/// window lands behind the app the user is working in. 0.1.19 raised it to the
/// status level, which covers ordinary windows — but a plain window can never
/// enter another app's full-screen Space, and many people work full screen, so
/// the reminder still went unseen. The window is therefore converted into a
/// non-activating panel (`ReminderPanel`) at the status level (above app
/// windows, below menus), allowed into every Space including full-screen ones,
/// kept out of ⌘` cycling, and ordered front without activating TimeTracker or
/// taking keyboard focus (see `build_or_focus`).
///
/// Windows needs none of this: `always_on_top` there is `HWND_TOPMOST`, which
/// already sits over other apps, full-screen ones included.
#[cfg(target_os = "macos")]
fn float_above_other_apps(win: &tauri::WebviewWindow) {
    use tauri::Manager;
    use tauri_nspanel::{CollectionBehavior, ManagerExt, PanelLevel, WebviewWindowExt};

    let app = win.app_handle();
    let panel = match app.get_webview_panel(WINDOW_LABEL) {
        Ok(panel) => panel,
        Err(_) => match win.to_panel::<ReminderPanel>() {
            Ok(panel) => {
                if let Err(e) =
                    panel.add_style_mask(objc2_app_kit::NSWindowStyleMask::NonactivatingPanel)
                {
                    tracing::warn!("reminder: could not make the panel non-activating: {e:?}");
                }
                panel.set_collection_behavior(
                    CollectionBehavior::new()
                        .can_join_all_spaces()
                        .full_screen_auxiliary()
                        .stationary()
                        .ignores_cycle()
                        .value(),
                );
                panel.set_floating_panel(true);
                // After set_floating_panel, which resets the level to "floating".
                panel.set_level(PanelLevel::Status.value());
                panel.set_hides_on_deactivate(false);
                panel
            }
            Err(e) => {
                tracing::warn!("reminder: could not turn the window into a panel: {e}");
                return;
            }
        },
    };
    panel.order_front_regardless();
}

#[cfg(not(target_os = "macos"))]
fn float_above_other_apps(_win: &tauri::WebviewWindow) {}

/// A `keep_raised` loop is running.
#[cfg(target_os = "macos")]
static KEEPING_RAISED: AtomicBool = AtomicBool::new(false);

/// How often an open reminder is raised again.
#[cfg(target_os = "macos")]
const RERAISE_EVERY: std::time::Duration = std::time::Duration::from_secs(2);

#[cfg(target_os = "macos")]
fn is_up() -> bool {
    PENDING.lock().map(|g| g.is_some()).unwrap_or(false)
}

/// macOS: follow the user to whichever Space they are on.
///
/// A panel that joins all Spaces still stays out of a full-screen Space the
/// user switches to *after* it appeared — seen on a Mac: raised on the desktop,
/// then gone once the user swiped back into a full-screen app. Ordering it
/// front again from that Space brings it there. So while a reminder is up, it
/// is raised again every couple of seconds (never focused — see
/// `build_or_focus`); the loop ends with the reminder.
#[cfg(target_os = "macos")]
fn keep_raised(app: &AppHandle) {
    if KEEPING_RAISED.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(RERAISE_EVERY).await;
            if !is_up() {
                KEEPING_RAISED.store(false, Ordering::SeqCst);
                // A reminder raised between the check and the store would have
                // found the flag set and started no loop of its own.
                if is_up() && !KEEPING_RAISED.swap(true, Ordering::SeqCst) {
                    continue;
                }
                return;
            }
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || {
                use tauri_nspanel::ManagerExt;
                if let Ok(panel) = handle.get_webview_panel(WINDOW_LABEL) {
                    panel.order_front_regardless();
                }
            });
        }
    });
}
