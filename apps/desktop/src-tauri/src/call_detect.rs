//! Call detection: is the employee on a Google Meet call or a Slack huddle?
//!
//! Someone listening on a call doesn't touch the keyboard or mouse, so without
//! this the idle timer would record most of every meeting as idle (and the
//! "You're idle" notice would pop up mid-call). Since 0.1.19 there is no manual
//! Meeting mode to switch that off; the app notices the call itself.
//!
//! The signal is the microphone: a call is in progress while Slack or a web
//! browser (where Google Meet runs) is recording from it. That survives the
//! person looking at another window during the call, which a window-title check
//! would not, and needs no extra permission on either platform:
//! - macOS 14+: CoreAudio lists the processes recording input, with bundle ids.
//!   On older macOS only "the default microphone is in use" is known, so any app
//!   counts there.
//! - Windows: the privacy settings record, per app, when it started and stopped
//!   using the microphone (`ConsentStore\microphone` in the user's registry).
//!
//! While a call is seen, [`IdleHandle::note_call`] holds idle off; idle counts
//! again from the moment the call ends. Screenshots keep being taken, and the
//! server tags the call ones `meeting` and leaves them unscored.

use std::time::Duration;

use crate::idle::IdleHandle;

/// How often to look. Far below the idle threshold, so a silent call never
/// crosses it between looks.
const POLL: Duration = Duration::from_secs(10);

/// Apps whose microphone use means a Meet call or Slack huddle. Matched as
/// lowercase substrings of a macOS bundle id or a Windows registry key name
/// (an exe path with `#` for `\`, or a Store package name).
const CALL_APPS: [&str; 18] = [
    "slack", // com.tinyspeck.slackmacgap, slack.exe, 91750D7E.Slack_…
    "com.google.chrome",
    "chrome.exe",
    "com.microsoft.edgemac",
    "msedge.exe",
    "com.brave.browser",
    "brave.exe",
    "company.thebrowser", // Arc
    "arc.exe",
    "org.mozilla.",
    "firefox.exe",
    "com.apple.safari",
    "com.apple.webkit.gpu", // Safari records the microphone in its GPU process
    "com.operasoftware",
    "opera.exe",
    "com.vivaldi",
    "vivaldi.exe",
    "chromium",
];

/// Is `app` (bundle id or registry key name) one that runs our calls?
fn is_call_app(app: &str) -> bool {
    let app = app.to_lowercase();
    CALL_APPS.iter().any(|p| app.contains(p))
}

/// The call app recording from the microphone right now, if any.
pub fn call_in_progress() -> Option<String> {
    match platform::mic_users() {
        Some(apps) => apps.into_iter().find(|a| is_call_app(a)),
        // The platform can't say which app: any microphone use counts.
        None => platform::mic_in_use().then(|| "microphone".to_string()),
    }
}

/// Background loop: keep idle off while a call is in progress.
pub async fn run(idle: IdleHandle) {
    let mut current: Option<String> = None;
    loop {
        tokio::time::sleep(POLL).await;
        let app = tokio::task::spawn_blocking(call_in_progress)
            .await
            .ok()
            .flatten();
        if app.is_some() {
            idle.note_call();
        }
        if app != current {
            match &app {
                Some(a) => tracing::info!(app = %a, "call detected: idle paused"),
                None => tracing::info!("call ended: idle counts again"),
            }
            current = app;
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::{c_char, c_void, CStr};
    use std::ptr;

    type AudioObjectId = u32;

    #[repr(C)]
    struct Address {
        selector: u32,
        scope: u32,
        element: u32,
    }

    #[link(name = "CoreAudio", kind = "framework")]
    extern "C" {
        fn AudioObjectGetPropertyDataSize(
            id: AudioObjectId,
            address: *const Address,
            qualifier_size: u32,
            qualifier: *const c_void,
            out_size: *mut u32,
        ) -> i32;
        fn AudioObjectGetPropertyData(
            id: AudioObjectId,
            address: *const Address,
            qualifier_size: u32,
            qualifier: *const c_void,
            io_size: *mut u32,
            out: *mut c_void,
        ) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringGetCString(s: *const c_void, buf: *mut c_char, len: isize, enc: u32) -> u8;
        fn CFRelease(cf: *const c_void);
    }

    const fn code(s: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*s)
    }
    const SYSTEM_OBJECT: AudioObjectId = 1;
    const SCOPE_GLOBAL: u32 = code(b"glob");
    const ELEMENT_MAIN: u32 = 0;
    const PROCESS_OBJECT_LIST: u32 = code(b"prs#"); // macOS 14+
    const PROCESS_IS_RUNNING_INPUT: u32 = code(b"piri");
    const PROCESS_BUNDLE_ID: u32 = code(b"pbid");
    const DEFAULT_INPUT_DEVICE: u32 = code(b"dIn ");
    const DEVICE_IS_RUNNING_SOMEWHERE: u32 = code(b"gone");
    const UTF8: u32 = 0x0800_0100;

    fn address(selector: u32) -> Address {
        Address {
            selector,
            scope: SCOPE_GLOBAL,
            element: ELEMENT_MAIN,
        }
    }

    /// A fixed-size property (`T` is a plain integer or object id).
    fn get<T: Copy + Default>(id: AudioObjectId, selector: u32) -> Option<T> {
        let mut out = T::default();
        let mut size = std::mem::size_of::<T>() as u32;
        // SAFETY: `out` is a valid, writable T and `size` is its exact size.
        let status = unsafe {
            AudioObjectGetPropertyData(
                id,
                &address(selector),
                0,
                ptr::null(),
                &mut size,
                (&mut out as *mut T).cast(),
            )
        };
        (status == 0).then_some(out)
    }

    fn process_objects() -> Option<Vec<AudioObjectId>> {
        let addr = address(PROCESS_OBJECT_LIST);
        let mut size = 0u32;
        // SAFETY: valid address and out-pointer; no qualifier.
        if unsafe {
            AudioObjectGetPropertyDataSize(SYSTEM_OBJECT, &addr, 0, ptr::null(), &mut size)
        } != 0
        {
            return None; // older macOS: no per-process list
        }
        let mut ids =
            vec![0 as AudioObjectId; size as usize / std::mem::size_of::<AudioObjectId>()];
        let mut size = (ids.len() * std::mem::size_of::<AudioObjectId>()) as u32;
        // SAFETY: `ids` has room for exactly `size` bytes.
        let status = unsafe {
            AudioObjectGetPropertyData(
                SYSTEM_OBJECT,
                &addr,
                0,
                ptr::null(),
                &mut size,
                ids.as_mut_ptr().cast(),
            )
        };
        if status != 0 {
            return None;
        }
        ids.truncate(size as usize / std::mem::size_of::<AudioObjectId>());
        Some(ids)
    }

    fn bundle_id(process: AudioObjectId) -> Option<String> {
        let mut s: *const c_void = ptr::null();
        let mut size = std::mem::size_of::<*const c_void>() as u32;
        // SAFETY: the property is a CFStringRef we own (and release) on success.
        let status = unsafe {
            AudioObjectGetPropertyData(
                process,
                &address(PROCESS_BUNDLE_ID),
                0,
                ptr::null(),
                &mut size,
                (&mut s as *mut *const c_void).cast(),
            )
        };
        if status != 0 || s.is_null() {
            return None;
        }
        let mut buf = [0 as c_char; 256];
        // SAFETY: `s` is a live CFString; `buf` is writable for its length.
        let ok = unsafe { CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as isize, UTF8) } != 0;
        // SAFETY: we own the reference returned by the property getter.
        unsafe { CFRelease(s) };
        if !ok {
            return None;
        }
        // SAFETY: CFStringGetCString wrote a NUL-terminated string on success.
        let id = unsafe { CStr::from_ptr(buf.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        (!id.is_empty()).then_some(id)
    }

    /// Bundle ids of the processes recording from any input device, or `None`
    /// where macOS can't list them (before 14).
    pub fn mic_users() -> Option<Vec<String>> {
        Some(
            process_objects()?
                .into_iter()
                .filter(|p| get::<u32>(*p, PROCESS_IS_RUNNING_INPUT).unwrap_or(0) != 0)
                .filter_map(bundle_id)
                .collect(),
        )
    }

    /// Every process CoreAudio knows about, as (bundle id, recording input).
    #[cfg(test)]
    pub fn audio_processes() -> Vec<(Option<String>, bool)> {
        process_objects()
            .unwrap_or_default()
            .into_iter()
            .map(|p| {
                let input = get::<u32>(p, PROCESS_IS_RUNNING_INPUT).unwrap_or(0) != 0;
                (bundle_id(p), input)
            })
            .collect()
    }

    /// Is the default microphone in use by anything (the pre-14 fallback)?
    pub fn mic_in_use() -> bool {
        get::<AudioObjectId>(SYSTEM_OBJECT, DEFAULT_INPUT_DEVICE)
            .filter(|d| *d != 0)
            .and_then(|d| get::<u32>(d, DEVICE_IS_RUNNING_SOMEWHERE))
            .is_some_and(|running| running != 0)
    }
}

#[cfg(windows)]
mod platform {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    const MICROPHONE: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";
    /// A "started, never stopped" entry older than this is a crashed app's
    /// leftover, not a call — otherwise one crash would switch idle off for good.
    const STALE: Duration = Duration::from_secs(12 * 3600);
    /// Seconds between the FILETIME epoch (1601) and the Unix epoch.
    const FILETIME_TO_UNIX: u64 = 11_644_473_600;

    fn in_use_now(app: &RegKey, now_unix: u64) -> bool {
        let start: u64 = app.get_value("LastUsedTimeStart").unwrap_or(0);
        let stop: u64 = app.get_value("LastUsedTimeStop").unwrap_or(0);
        if start == 0 || stop != 0 {
            return false;
        }
        let started_unix = (start / 10_000_000).saturating_sub(FILETIME_TO_UNIX);
        now_unix.saturating_sub(started_unix) < STALE.as_secs()
    }

    /// Registry names of the apps using the microphone right now: Store
    /// packages directly under the key, desktop apps under `NonPackaged`.
    pub fn mic_users() -> Option<Vec<String>> {
        let root = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(MICROPHONE)
            .ok()?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
        let mut users = Vec::new();
        let mut scan = |parent: &RegKey| {
            for name in parent.enum_keys().flatten() {
                if name == "NonPackaged" {
                    continue;
                }
                if let Ok(app) = parent.open_subkey(&name) {
                    if in_use_now(&app, now) {
                        users.push(name);
                    }
                }
            }
        };
        scan(&root);
        if let Ok(desktop) = root.open_subkey("NonPackaged") {
            scan(&desktop);
        }
        Some(users)
    }

    pub fn mic_in_use() -> bool {
        false // mic_users always answers on Windows
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    pub fn mic_users() -> Option<Vec<String>> {
        Some(Vec::new()) // not detected on Linux
    }

    pub fn mic_in_use() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meet_and_slack_hosts_are_call_apps() {
        for app in [
            "com.tinyspeck.slackmacgap",
            "com.google.Chrome.helper",
            "com.apple.WebKit.GPU",
            "company.thebrowser.Browser",
            r"C:#Program Files#Google#Chrome#Application#chrome.exe",
            r"C:#Users#a#AppData#Local#slack#app-4.41.105#slack.exe",
            "91750D7E.Slack_8she8kybcnzg4",
            r"C:#Program Files (x86)#Microsoft#Edge#Application#msedge.exe",
        ] {
            assert!(is_call_app(app), "{app} should count");
        }
    }

    #[test]
    fn other_microphone_users_are_not_calls() {
        for app in [
            "com.apple.VoiceMemos",
            "com.apple.corespeechd",
            "us.zoom.xos",
            r"C:#Windows#System32#SoundRecorder.exe",
            "Microsoft.WindowsSoundRecorder_8wekyb3d8bbwe",
        ] {
            assert!(!is_call_app(app), "{app} should not count");
        }
    }

    /// Prints what this machine reports right now. `cargo test -- --ignored
    /// --nocapture call_detect` while on a call to see it work.
    #[test]
    #[ignore]
    fn report_this_machine() {
        #[cfg(target_os = "macos")]
        println!("audio processes: {:?}", platform::audio_processes());
        println!("mic users: {:?}", platform::mic_users());
        println!("default mic in use: {}", platform::mic_in_use());
        println!("call in progress: {:?}", call_in_progress());
    }
}
