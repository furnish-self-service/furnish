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

use furnish::selfupdate::{self, Channel, Method, SelfUpdate};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

/// What the window shows in Settings → About and the sidebar.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// How furnish was installed.
    pub method: Method,
    pub channel: Channel,
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
type Prepared = (tauri_plugin_updater::Update, Vec<u8>);
// Serialize feed checks, replacements and installs so apply cannot take superseded bytes.
static PENDING: tauri::async_runtime::Mutex<Option<Prepared>> = tauri::async_runtime::Mutex::const_new(None);

/// How furnish was installed doesn't change while it runs.
fn method() -> Method {
    static METHOD: OnceLock<Method> = OnceLock::new();
    *METHOD.get_or_init(selfupdate::detect)
}

fn fresh() -> Status {
    let effective = furnish::settings::effective();
    let method = method();
    let current = crate::build_info::current().version;
    Status {
        channel: Channel::selected(&current, method, effective.settings.update_channel),
        method,
        mode: selfupdate::effective(effective.settings.self_update, method),
        locked: effective.locked.iter().any(|k| k == "selfUpdate"),
        current,
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
    if status.channel != now.channel { *status = now.clone(); }
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
    let mut pending = PENDING.lock().await;
    refresh(&app, download, &mut pending).await
}

pub async fn set_channel(app: &AppHandle, value: serde_json::Value) -> Result<(), String> {
    let mut pending = PENDING.lock().await;
    if !method().updates_itself() { return Err("This installation follows its package source.".into()); }
    crate::blocking(move || furnish::settings::set("updateChannel", value)).await?;
    *pending = None;
    change(app, |s| *s = fresh());
    Ok(())
}

async fn refresh(app: &AppHandle, download: bool, pending: &mut Option<Prepared>) -> Status {
    let before = status();
    if pending.as_ref().is_some_and(|(u, _)| selfupdate::semver(&u.version).is_none_or(|v| Channel::of(&v) != before.channel)) {
        *pending = None;
    }
    if before.mode == SelfUpdate::Off { return before; }
    change(app, |s| s.state = State::Checking);
    let current = before.current.clone();
    let found = tauri::async_runtime::spawn_blocking(move || selfupdate::check(&current, before.channel)).await;
    let available = match found {
        Ok(Ok(available)) => available,
        result => {
            eprintln!("note: no update check: {result:?}");
            // Offline: an already verified download from the active channel is still usable.
            if fresh().channel != before.channel { *pending = None; }
            return change(app, |s| s.state = if pending.is_some() { State::Ready } else { State::Idle });
        }
    };
    let prepared_version = pending.as_ref().and_then(|(u, _)| selfupdate::semver(&u.version));
    let offered_version = available.as_ref().and_then(|a| selfupdate::semver(&a.version));
    let same = prepared_version.is_some() && prepared_version == offered_version;
    let mode = status();
    if mode.channel != before.channel { *pending = None; return mode; }
    let permitted = mode.method.updates_itself() && mode.mode != SelfUpdate::Off
        && (!mode.locked || mode.mode == SelfUpdate::Auto);
    let wanted = download || mode.mode == SelfUpdate::Auto;
    if !same || !permitted { *pending = None; }
    let state = if pending.is_some() { State::Ready }
        else if permitted && wanted && available.is_some() { State::Downloading }
        else { State::Idle };
    let status = change(app, |s| {
        s.available = available.as_ref().map(|a| a.version.clone());
        s.notes = available.as_ref().and_then(|a| a.notes.clone());
        s.state = state;
        s.error = None;
    });
    let Some(offer) = available.filter(|_| state == State::Downloading) else { return status };
    match fetch(app, before.channel, &offer.version).await {
        Ok(Some(prepared)) => {
            if fresh().channel != before.channel { return change(app, |_| {}); }
            let version = selfupdate::semver(&prepared.0.version).map(|v| selfupdate::display(&v));
            let notes = prepared.0.body.clone();
            *pending = Some(prepared);
            change(app, |s| {
                s.available = version;
                s.notes = notes;
                s.state = State::Ready;
            })
        }
        Ok(None) => change(app, |s| {
            s.available = None;
            s.notes = None;
            s.state = State::Idle;
        }),
        Err(err) => change(app, |s| {
            s.state = State::Idle;
            s.error = Some(err);
        }),
    }
}

/// Download the update with Tauri's updater, which checks it against the key and the version.
async fn fetch(app: &AppHandle, channel: Channel, offered: &str) -> Result<Option<Prepared>, String> {
    let current = selfupdate::semver(&crate::build_info::current().version).ok_or("invalid installed version")?;
    let url = selfupdate::feed_url(channel);
    if !selfupdate::feed_allowed(&url) {
        return Err(format!("the update feed {url} doesn't use HTTPS"));
    }
    let updater = app
        .updater_builder()
        .version_comparator(move |_, release| channel.offers(&current, &release.version))
        .endpoints(vec![url.parse().map_err(|e| format!("{url}: {e}"))?])
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else { return Ok(None) };
    // The manifest can change between the lightweight check and the updater's signed download.
    if selfupdate::semver(&update.version) < selfupdate::semver(offered) {
        return Err("The update feed changed during the download. Check again.".into());
    }
    let bytes = update.download(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
    Ok(Some((update, bytes)))
}

/// Install the downloaded update and start the new version: refused while an install runs, and
/// with `only_hidden` while the window is open (automatic updates don't interrupt anyone). On
/// Windows the installer replaces furnish and starts it again (this process ends here).
pub async fn apply(app: AppHandle, only_hidden: bool) -> Result<bool, String> {
    let mut pending = PENDING.lock().await;
    if pending.is_none() { return Ok(false); }
    if !only_hidden {
        // The feed may have advanced since the last interval while the window was open.
        let checked = refresh(&app, true, &mut pending).await;
        if let Some(error) = checked.error { return Err(error); }
    }
    let now = status();
    if pending.as_ref().is_some_and(|(u, _)| selfupdate::semver(&u.version).is_none_or(|v| Channel::of(&v) != now.channel)) {
        *pending = None;
        change(&app, |_| {});
        return Ok(false);
    }
    if now.mode == SelfUpdate::Off || ((only_hidden || now.locked) && now.mode != SelfUpdate::Auto) {
        return Ok(false);
    }
    if only_hidden && app.get_webview_window("main").is_some_and(|w| w.is_visible().unwrap_or(false)) {
        return Ok(false);
    }
    let Some(idle) = crate::tray::IdleRestart::reserve(&app) else { return Ok(false) };
    let Some((update, bytes)) = pending.take() else { return Ok(false) };
    // Off the main thread: macOS may ask for an administrator's password.
    let result = tauri::async_runtime::spawn_blocking(move || update.install(&bytes)).await;
    if let Err(message) = result.map_err(|e| e.to_string()).and_then(|res| res.map_err(|e| e.to_string())) {
        change(&app, |s| {
            s.state = State::Idle;
            s.error = Some(message.clone());
        });
        return Err(message);
    }
    idle.commit();
    app.request_restart();
    Ok(true)
}
