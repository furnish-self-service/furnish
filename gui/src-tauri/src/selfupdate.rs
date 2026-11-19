//! furnish updating itself (see `furnish::selfupdate`): the window asks for a check on launch and
//! with the update interval. Where furnish installs its own updates (Windows MSI, macOS app from
//! the .dmg) and `selfUpdate` is `auto`, the new version is downloaded and checked right away,
//! then installed and started when nothing is installing: by the window once it is closed to the
//! tray, or when someone chooses Restart to update. Elsewhere furnish only says it is out.
//!
//! Downloading and installing is Tauri's updater with the feed from `furnish::selfupdate` and the
//! key in tauri.conf.json (`requireSignedVersion`: the signature must name the version the feed
//! announces). tauri.conf.json allows non-HTTPS feeds (`dangerousInsecureTransportProtocol`)
//! because furnish checks the feed itself (`feed_allowed`: HTTPS, or HTTP on this computer for
//! tests); the downloads are verified against the key whatever their transport.

use std::sync::{Mutex, OnceLock};

use furnish::selfupdate::{self, Method, SelfUpdate};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

/// What the window shows in Settings → About and the sidebar.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// How furnish was installed.
    pub method: Method,
    /// The `selfUpdate` setting as it applies here (`auto` only where furnish installs updates).
    pub mode: SelfUpdate,
    /// The admin policy locks `selfUpdate`.
    pub locked: bool,
    /// This version, as furnish shows versions.
    pub current: String,
    /// A newer version, if the last check found one.
    pub available: Option<String>,
    /// A link to what changed in it.
    pub notes: Option<String>,
    pub state: State,
    /// Why the last download or install failed (checks fail silently: offline is normal).
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    Idle,
    Checking,
    Downloading,
    /// Downloaded and checked: installs and restarts when nothing is installing.
    Ready,
}

static STATUS: Mutex<Option<Status>> = Mutex::new(None);
/// The downloaded update, checked against its signature, waiting to be installed.
static PENDING: Mutex<Option<(tauri_plugin_updater::Update, Vec<u8>)>> = Mutex::new(None);

/// How furnish was installed doesn't change while it runs.
fn method() -> Method {
    static METHOD: OnceLock<Method> = OnceLock::new();
    *METHOD.get_or_init(selfupdate::detect)
}

fn fresh() -> Status {
    let effective = furnish::settings::effective();
    let method = method();
    Status {
        method,
        mode: selfupdate::effective(effective.settings.self_update, method),
        locked: effective.locked.iter().any(|k| k == "selfUpdate"),
        current: crate::build_info::current().version,
        available: None,
        notes: None,
        state: State::Idle,
        error: None,
    }
}

/// The status, with the setting as it is now (it may have changed since).
pub fn status() -> Status {
    let mut guard = STATUS.lock().unwrap_or_else(|e| e.into_inner());
    let now = fresh();
    let status = guard.get_or_insert_with(|| now.clone());
    status.mode = now.mode;
    status.locked = now.locked;
    status.clone()
}

fn change(app: &AppHandle, f: impl FnOnce(&mut Status)) -> Status {
    let mut status = status();
    f(&mut status);
    *STATUS.lock().unwrap_or_else(|e| e.into_inner()) = Some(status.clone());
    let _ = app.emit("furnish://self-update", &status);
    crate::tray::set_update_ready(app, status.state == State::Ready);
    status
}

/// Ask the feed for a newer version; with `download` (or `selfUpdate` `auto`) fetch it where
/// furnish installs its own updates. Offline or a feed that doesn't answer: nothing to show.
pub async fn check(app: AppHandle, download: bool) -> Status {
    let before = status();
    if before.mode == SelfUpdate::Off || matches!(before.state, State::Checking | State::Downloading | State::Ready) {
        return before;
    }
    change(&app, |s| s.state = State::Checking);
    let current = before.current.clone();
    let found = tauri::async_runtime::spawn_blocking(move || selfupdate::check(&current)).await;
    let available = match found {
        Ok(Ok(available)) => available,
        Ok(Err(err)) => {
            eprintln!("note: no update check: {err:#}");
            return change(&app, |s| s.state = State::Idle);
        }
        Err(_) => return change(&app, |s| s.state = State::Idle),
    };
    let status = change(&app, |s| {
        s.state = State::Idle;
        s.available = available.as_ref().map(|a| a.version.clone());
        s.notes = available.as_ref().and_then(|a| a.notes.clone());
    });
    let wanted = download || status.mode == SelfUpdate::Auto;
    if status.available.is_none() || !wanted || !status.method.updates_itself() || (status.locked && status.mode != SelfUpdate::Auto) {
        return status;
    }
    change(&app, |s| {
        s.state = State::Downloading;
        s.error = None;
    });
    match fetch(&app).await {
        Ok(Some(pending)) => {
            *PENDING.lock().unwrap_or_else(|e| e.into_inner()) = Some(pending);
            change(&app, |s| s.state = State::Ready)
        }
        Ok(None) => change(&app, |s| s.state = State::Idle),
        Err(err) => change(&app, |s| {
            s.state = State::Idle;
            s.error = Some(err);
        }),
    }
}

/// Download the update with Tauri's updater, which checks it against the key and the version.
async fn fetch(app: &AppHandle) -> Result<Option<(tauri_plugin_updater::Update, Vec<u8>)>, String> {
    let current = crate::build_info::current().version;
    let url = selfupdate::feed_url(&current);
    if !selfupdate::feed_allowed(&url) {
        return Err(format!("the update feed {url} doesn't use HTTPS"));
    }
    let updater = app
        .updater_builder()
        .endpoints(vec![url.parse().map_err(|e| format!("{url}: {e}"))?])
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else { return Ok(None) };
    let bytes = update.download(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
    Ok(Some((update, bytes)))
}

/// Install the downloaded update and start the new version: refused while an install runs, and
/// with `only_hidden` while the window is open (automatic updates don't interrupt anyone). On
/// Windows the installer replaces furnish and starts it again (this process ends here).
pub fn apply(app: &AppHandle, only_hidden: bool) -> Result<bool, String> {
    if only_hidden && app.get_webview_window("main").is_some_and(|w| w.is_visible().unwrap_or(false)) {
        return Ok(false);
    }
    let Some(_idle) = crate::tray::IdleRestart::reserve(app) else { return Ok(false) };
    let Some((update, bytes)) = PENDING.lock().unwrap_or_else(|e| e.into_inner()).take() else { return Ok(false) };
    if let Err(err) = update.install(&bytes) {
        let message = err.to_string();
        change(app, |s| {
            s.state = State::Idle;
            s.error = Some(message.clone());
        });
        return Err(message);
    }
    app.restart();
}
