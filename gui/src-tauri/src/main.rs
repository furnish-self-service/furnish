// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! furnish GUI backend: thin Tauri commands over the same library the CLI uses.

mod app_icon;
mod build_info;
#[cfg(target_os = "linux")]
mod desktop_entry;
mod autostart;
mod desktop_update;
mod selfupdate;
mod tray;
mod tray_icon;

use std::{
    fs,
    io::Read as _,
    path::Path,
};

use base64::Engine as _;
use furnish::{
    catalog::{self, App},
    engine, options,
};
use serde::Serialize;
use tauri::{Emitter, Manager as _, async_runtime::spawn_blocking};

type CmdResult<T> = Result<T, String>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    /// `collection/id`: unique across collections.
    key: String,
    id: String,
    collection: String,
    cached_check: Option<serde_json::Value>,
    /// The name to show (`metadata.furnish.name`), if other than the id.
    name: Option<String>,
    description: String,
    categories: Vec<String>,
    /// Data URL of the app's icon, if the collection provides one.
    icon: Option<String>,
    /// Data URL of the icon for dark mode (`metadata.furnish.iconDark`).
    icon_dark: Option<String>,
    resources: Vec<ResourceInfo>,
    /// The DSC configuration document as written in the collection, for experts to inspect.
    source: String,
    /// Apps installed first: required ones always, optional ones if chosen (for the current options).
    dependencies: Vec<ItemInfo>,
    /// Required apps with the option values used for the installed configuration.
    installed_requirements: Vec<String>,
    /// Options the app declares, with their current values.
    options: Vec<options::Resolved>,
    /// Changed options and their installed values (null when the option was not recorded).
    option_changes: serde_json::Map<String, serde_json::Value>,
    /// Recommended for this user (`metadata.furnish.recommended`).
    recommended: bool,
    /// Required collection options nobody answered yet: installing waits for them.
    unanswered: Vec<String>,
    /// Held at this earlier version: the collection's current one failed, or the person chose it.
    held: Option<HeldInfo>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HeldInfo {
    #[serde(flatten)]
    version: catalog::Version,
    /// The person chose this version (it stays until they choose another).
    chosen: bool,
    /// The collection has another version of the app now.
    newer: bool,
}

#[derive(Serialize)]
struct ResourceInfo {
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct KitInfo {
    /// `collection/id`, like app keys (kits and apps share a namespace).
    key: String,
    id: String,
    collection: String,
    name: Option<String>,
    description: String,
    categories: Vec<String>,
    icon: Option<String>,
    icon_dark: Option<String>,
    recommended: bool,
    items: Vec<ItemInfo>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ItemInfo {
    /// Key of the app this item installs.
    key: String,
    required: bool,
    /// Preselected when installing (required items always are).
    selected: bool,
}

#[derive(Serialize)]
struct CollectionInfo {
    name: String,
    /// Provided by the admin policy: can't be removed.
    managed: bool,
    url: String,
    revision: String,
    apps: usize,
    /// Options shared by the collection's apps; required ones without a value are `missing`.
    options: Vec<options::Resolved>,
    /// Data URL of the collection's avatar (`icon` in collection.yaml).
    icon: Option<String>,
    /// Data URL of the avatar for dark mode (`iconDark`).
    #[serde(rename = "iconDark")]
    icon_dark: Option<String>,
    /// History kept, downloads and falling back, and which of them the organization sets.
    settings: furnish::settings::CollectionEffective,
    /// The language of the collection's own texts, and its translations (`locales/`).
    language: String,
    locales: std::collections::BTreeMap<String, furnish::locale::LocaleFile>,
    /// The server had a newer commit than the one checked out, when furnish last looked.
    #[serde(rename = "updateAvailable")]
    update_available: bool,
}

/// The organization's branding (policy `branding`), checked: colors are `#rgb`/`#rrggbb`, theme
/// color names letters and dashes, so nothing else reaches the page's CSS.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BrandingInfo {
    name: String,
    /// Data URL of the logo, if any.
    logo: Option<String>,
    accent: Option<String>,
    accent_dark: Option<String>,
    light: std::collections::BTreeMap<String, String>,
    dark: std::collections::BTreeMap<String, String>,
}

fn hex_color(value: &str) -> Option<String> {
    let hex = value.strip_prefix('#')?;
    (matches!(hex.len(), 3 | 6) && hex.chars().all(|c| c.is_ascii_hexdigit())).then(|| value.to_string())
}

fn branding_info() -> BrandingInfo {
    let b = furnish::policy::load().branding;
    let colors = |m: std::collections::BTreeMap<String, String>| {
        m.into_iter()
            .filter_map(|(name, value)| {
                let ok = !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == '-');
                match (ok, hex_color(&value)) {
                    (true, Some(v)) => Some((name, v)),
                    _ => {
                        eprintln!("warning: ignoring branding color {name}: {value} (use a name like `canvas` and #rrggbb)");
                        None
                    }
                }
            })
            .collect()
    };
    BrandingInfo {
        name: b.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "furnish".into()),
        logo: catalog::branding_logo().as_deref().and_then(icon_data_url),
        accent: b.accent.as_deref().and_then(hex_color),
        accent_dark: b.accent_dark.as_deref().and_then(hex_color),
        light: colors(b.theme.light),
        dark: colors(b.theme.dark),
    }
}

/// The furnish:// link furnish was started with, if any (later ones arrive as events).
#[tauri::command]
fn pending_link(app: tauri::AppHandle) -> Option<String> {
    use tauri_plugin_deep_link::DeepLinkExt;
    app.deep_link().get_current().ok().flatten().and_then(|urls| urls.first().map(|u| u.to_string()))
}

/// The branding, for the page. The window's title and icon and the tray icon follow it here: the
/// page asks at start and whenever the policy or an organization's description changed.
#[tauri::command]
fn get_branding(app: tauri::AppHandle) -> BrandingInfo {
    let info = branding_info();
    app_icon::refresh(&app, &info.name);
    tray::refresh_icon(&app);
    info
}

#[derive(Serialize)]
struct Revisions {
    old: String,
    new: String,
}

/// Run blocking work (git, DSC) off the UI thread; errors become display strings.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> anyhow::Result<T> + Send + 'static) -> CmdResult<T> {
    spawn_blocking(work)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| if e.is::<engine::Cancelled>() { engine::CANCELLED.to_string() } else { format!("{e:#}") })
}

fn app_info(app: &App) -> AppInfo {
    let options = app.options();
    let current_values = options::values(&options);
    let recorded_values = app.recorded_values();
    // Only what applies here: experts find the skipped ones in the steps preview.
    let resources = app
        .resources_here()
        .iter()
        .map(|r| ResourceInfo { name: r["name"].as_str().unwrap_or_default().to_string() })
        .collect();
    AppInfo {
        key: app.key(),
        cached_check: app.cached_check(),
        id: app.id.clone(),
        collection: app.collection.clone(),
        name: app.name.clone(),
        description: app.description.clone(),
        categories: app.categories.clone(),
        icon: app.icon.as_deref().and_then(icon_data_url),
        icon_dark: app.icon_dark.as_deref().and_then(icon_data_url),
        resources,
        source: fs::read_to_string(&app.path).unwrap_or_default(),
        dependencies: items(&app.collection, &app.dependencies_for(&current_values)),
        installed_requirements: app
            .dependencies_for(recorded_values.as_ref().unwrap_or(&current_values))
            .into_iter()
            .filter(|dep| dep.required)
            .map(|dep| format!("{}/{}", app.collection, dep.app))
            .collect(),
        option_changes: options::changed_values(&options, recorded_values.as_ref()),
        options,
        recommended: app.recommended(),
        unanswered: app.unanswered(),
        held: app.held_version().map(|version| HeldInfo {
            version,
            chosen: app.held.as_ref().is_some_and(|h| h.chosen),
            newer: app.newer_than_held(),
        }),
    }
}

fn items(collection: &str, items: &[catalog::Item]) -> Vec<ItemInfo> {
    items
        .iter()
        .map(|i| ItemInfo { key: format!("{collection}/{}", i.app), required: i.required, selected: i.required || i.selected })
        .collect()
}

#[tauri::command]
async fn description_image(collection: String, path: String) -> CmdResult<Option<String>> {
    blocking(move || {
        let collection = catalog::get(&collection)?;
        Ok(collection.description_image(&path).as_deref().and_then(|path| image_data_url(path, 8 * 1024 * 1024)))
    })
    .await
}

#[tauri::command]
async fn open_description_link(url: String) -> CmdResult<()> {
    blocking(move || furnish::endpoint::open_browser(&url)).await
}

fn icon_data_url(path: &Path) -> Option<String> {
    image_data_url(path, 1024 * 1024)
}

fn image_data_url(path: &Path, limit: u64) -> Option<String> {
    let mime = match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => return None,
    };
    // Size first: the collection decides what the file is, and reading it all could be costly.
    if fs::metadata(path).ok()?.len() > limit {
        return None;
    }
    let mut bytes = Vec::new();
    fs::File::open(path).ok()?.take(limit + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > limit {
        return None;
    }
    Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

/// Best effort: a collection whose git metadata can't be read still shows up (as "unknown"),
/// so one broken repository doesn't hide the others.
fn collection_info(c: &catalog::Collection) -> anyhow::Result<CollectionInfo> {
    let unknown = |_| "unknown".to_string();
    let locales = c.locales();
    Ok(CollectionInfo {
        name: c.name.clone(),
        managed: c.managed,
        url: c.url().unwrap_or_else(unknown),
        revision: c.revision().unwrap_or_else(unknown),
        apps: c.apps().len(),
        options: c.options()?,
        icon: c.icon().as_deref().and_then(icon_data_url),
        icon_dark: c.icon_dark().as_deref().and_then(icon_data_url),
        settings: furnish::settings::for_collection(&c.name),
        language: locales.language,
        locales: locales.files,
        update_available: c.update_available(),
    })
}

#[tauri::command]
async fn list_apps() -> CmdResult<Vec<AppInfo>> {
    blocking(|| Ok(catalog::all_apps()?.iter().map(app_info).collect())).await
}

#[tauri::command]
async fn list_kits() -> CmdResult<Vec<KitInfo>> {
    blocking(|| {
        Ok(catalog::all_kits()?
            .into_iter()
            .map(|k| KitInfo {
                key: format!("{}/{}", k.collection, k.id),
                recommended: k.recommended(),
                items: items(&k.collection, &k.items),
                icon: k.icon.as_deref().and_then(icon_data_url),
                icon_dark: k.icon_dark.as_deref().and_then(icon_data_url),
                id: k.id,
                collection: k.collection,
                name: k.name,
                description: k.description,
                categories: k.categories,
            })
            .collect())
    })
    .await
}

#[tauri::command]
async fn app_status(key: String, revision: Option<String>) -> CmdResult<engine::Check> {
    blocking(move || {
        let app = catalog::find_app(&key)?;
        let check = app.check()?;
        app.remember_check(revision.as_deref(), &check);
        Ok(check)
    }).await
}

/// A step progress event for the UI, tagged with the app it belongs to.
#[derive(Clone, Serialize)]
struct ProgressEvent {
    key: String,
    #[serde(flatten)]
    progress: engine::Progress,
}

#[derive(Clone, Serialize)]
struct OperationRevision<'a> {
    key: &'a str,
    event: &'static str,
    revision: Option<String>,
}

#[derive(Clone)]
enum Action {
    Install(catalog::Which),
    Uninstall,
    Reinstall,
}

/// An install that failed, after which furnish installed an earlier version (fallback "automatic").
#[derive(Serialize)]
struct FellBack {
    date: String,
    error: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OperationResult {
    requires_new_session: bool,
    fallback: Option<FellBack>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    waiting_for_close: Vec<String>,
}

/// Run an install/update, uninstall or reinstall, streaming step progress as `furnish://progress` events.
/// `trigger`: who started it, for reports: `update` (automatic updates), `autoInstall`, else the user.
async fn run(handle: tauri::AppHandle, key: String, action: Action, trigger: Option<String>) -> CmdResult<OperationResult> {
    let trigger = match trigger.as_deref() {
        Some("update") => "update",
        Some("autoInstall") => "autoInstall",
        _ => "user",
    };
    let running = tray::Running::start(&handle).ok_or("furnish is quitting")?;
    let result = blocking(move || furnish::reports::with_trigger(trigger, || {
        let app = catalog::find_app(&key)?;
        let record_revision = |revision| {
            let _ = handle.emit("furnish://progress", OperationRevision { key: &key, event: "revision", revision });
        };
        let target = match &action {
            Action::Install(which) => which,
            Action::Reinstall | Action::Uninstall => &catalog::Which::Shown,
        };
        record_revision(app.operation_revision(target));
        let report = |progress| {
            let _ = handle.emit("furnish://progress", ProgressEvent { key: key.clone(), progress });
        };
        let install = |which| engine::session_result(report, |progress| app.install_version(which, progress));
        let (requires_new_session, fallback) = match action {
            Action::Install(which) => match install(which.clone()) {
                Err(err) if which == catalog::Which::Shown && !err.is::<engine::Cancelled>() && !err.is::<engine::WaitingForClose>() && app.held.is_none() => {
                    let automatic = furnish::settings::for_collection(&app.collection).settings.fallback == furnish::settings::Fallback::Automatic;
                    let Some(version) = app.previous_version().ok().filter(|_| automatic) else { return Err(err) };
                    record_revision(Some(version.commit.clone()));
                    (install(catalog::Which::Previous)?, Some(FellBack { date: version.date, error: format!("{err:#}") }))
                }
                result => (result?, None),
            },
            Action::Uninstall => (engine::session_result(report, |progress| app.uninstall(progress))?, None),
            Action::Reinstall => (engine::session_result(report, |progress| app.reinstall(progress))?, None),
        };
        Ok(OperationResult { requires_new_session, fallback, waiting_for_close: vec![] })
    }).or_else(|err: anyhow::Error| {
        let waiting = err.downcast::<engine::WaitingForClose>()?;
        Ok(OperationResult { requires_new_session: false, fallback: None, waiting_for_close: waiting.0 })
    }))
    .await;
    std::thread::spawn(furnish::reports::send);
    drop(running); // quits now if Quit was chosen while this ran
    result
}

/// Install or update; with the fallback "automatic", a failure installs the previous version.
#[tauri::command]
async fn install(app: tauri::AppHandle, key: String, trigger: Option<String>) -> CmdResult<OperationResult> {
    run(app, key, Action::Install(catalog::Which::Shown), trigger).await
}

/// Install the previous version of an app (see `previous_version`).
#[tauri::command]
async fn install_previous(app: tauri::AppHandle, key: String, trigger: Option<String>) -> CmdResult<OperationResult> {
    run(app, key, Action::Install(catalog::Which::Previous), trigger).await
}

/// For an app held at an earlier version: install the collection's current one.
#[tauri::command]
async fn install_current(app: tauri::AppHandle, key: String, trigger: Option<String>) -> CmdResult<OperationResult> {
    run(app, key, Action::Install(catalog::Which::Current), trigger).await
}

/// Install a version of an app the person chose (a commit, see `versions`) and keep it there.
#[tauri::command]
async fn install_version(app: tauri::AppHandle, key: String, commit: String, trigger: Option<String>) -> CmdResult<OperationResult> {
    run(app, key, Action::Install(catalog::Which::Version(commit)), trigger).await
}

/// The versions of an app the person can choose from, newest first; an error says why there
/// are none (e.g. the organization turned choosing off).
#[tauri::command]
async fn app_versions(key: String) -> CmdResult<Vec<catalog::Version>> {
    blocking(move || catalog::find_app(&key)?.versions()).await
}

/// The earlier version furnish would fall back to; an error says why there is none.
#[tauri::command]
async fn previous_version(key: String) -> CmdResult<catalog::Version> {
    blocking(move || catalog::find_app(&key)?.previous_version()).await
}

#[tauri::command]
async fn uninstall(app: tauri::AppHandle, key: String, trigger: Option<String>) -> CmdResult<OperationResult> {
    run(app, key, Action::Uninstall, trigger).await
}

/// Uninstall with the options the app was installed with, then install with the current ones.
#[tauri::command]
async fn reinstall(app: tauri::AppHandle, key: String, trigger: Option<String>) -> CmdResult<OperationResult> {
    run(app, key, Action::Reinstall, trigger).await
}

#[tauri::command]
fn build_info() -> build_info::BuildInfo {
    build_info::current()
}

/// How furnish was installed and whether a newer version of it is out or ready to install.
#[tauri::command]
fn self_update_status() -> selfupdate::Status {
    selfupdate::status()
}

#[cfg(target_os = "macos")]
#[tauri::command]
fn cli_link_installed() -> bool {
    let link = Path::new("/usr/local/bin/furnish");
    link.is_symlink() && link.canonicalize().is_ok_and(|target| target == Path::new("/Applications/furnish.app/Contents/MacOS/furnish"))
}

/// Make the CLI in the installed macOS app available to Terminal, on explicit request.
#[cfg(target_os = "macos")]
#[tauri::command]
async fn install_cli_link() -> CmdResult<()> {
    blocking(|| {
        let bundle = Path::new("/Applications/furnish.app");
        let exe = std::env::current_exe()?.canonicalize()?;
        anyhow::ensure!(
            furnish::selfupdate::app_bundle(&exe).as_deref() == Some(bundle),
            "Move furnish.app to /Applications and open it there."
        );
        let output = std::process::Command::new("/bin/sh").arg(bundle.join("Contents/Resources/install-cli.sh")).output()?;
        anyhow::ensure!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr).trim());
        Ok(())
    })
    .await
}

/// Ask the feed for a newer version of furnish; `download`: fetch it even if `selfUpdate` is
/// `notify` (someone chose Update now).
#[tauri::command]
async fn check_self_update(app: tauri::AppHandle, download: bool) -> selfupdate::Status {
    selfupdate::check(app, download).await
}

/// Install the downloaded version and restart into it, unless something is installing (or, with
/// `onlyHidden`, the window is open). Returns whether it did; on success furnish restarts. Off
/// the main thread: on macOS the updater may ask there for an administrator's password.
#[tauri::command]
async fn apply_self_update(app: tauri::AppHandle, only_hidden: bool) -> CmdResult<bool> {
    selfupdate::apply(app, only_hidden).await
}

/// Show the open-source components and their licenses: the installer's copy, else the website's.
#[tauri::command]
fn open_licenses(app: tauri::AppHandle) -> CmdResult<()> {
    use tauri::Manager;
    let bundled = app.path().resolve("THIRD-PARTY-LICENSES.html", tauri::path::BaseDirectory::Resource).ok().filter(|p| p.is_file());
    match bundled {
        Some(path) => furnish::endpoint::open_file(&path),
        None => furnish::endpoint::open_browser("https://furnish-self-service.github.io/licenses.html"),
    }
    .map_err(|e| format!("{e:#}"))
}

/// Change one setting for one collection; `null` returns it to the general setting.
#[tauri::command]
async fn set_collection_setting(name: String, key: String, value: serde_json::Value) -> CmdResult<()> {
    blocking(move || {
        if key == "tracking" {
            let tracking = serde_json::from_value::<Option<furnish::settings::Tracking>>(value)?;
            catalog::get(&name)?.set_tracking(tracking.as_ref())?;
            return Ok(());
        }
        furnish::settings::set_for_collection(&name, &key, value)?;
        if matches!(key.as_str(), "checkoutDepth" | "blobsOnDemand") {
            catalog::get(&name)?.update()?; // applied when git next talks to the server
        }
        Ok(())
    })
    .await
}

/// Set (or with `None`, reset) an option for a collection or an app (`collection/app`).
#[tauri::command]
async fn set_option(target: String, name: String, value: Option<serde_json::Value>) -> CmdResult<()> {
    blocking(move || {
        let resolved = match target.split_once('/') {
            Some(_) => catalog::find_app(&target)?.options(),
            None => options::resolve_collection(&target, &options::collection_declarations(&catalog::get(&target)?.dir)?),
        };
        let r = resolved.iter().find(|r| r.name == name).ok_or_else(|| anyhow::anyhow!("{target} has no option '{name}'"))?;
        options::set(&target, &name, value, &r.declaration)
    })
    .await
}

/// Preview installing (or uninstalling) without changing anything (DSC what-if).
#[tauri::command]
async fn plan(key: String, uninstall: bool) -> CmdResult<Vec<engine::Step>> {
    blocking(move || catalog::find_app(&key)?.plan(uninstall)).await
}

/// Stop the running install/update/uninstall after its current step.
#[tauri::command]
fn abort() {
    engine::abort();
    if let Err(err) = furnish::service::stop(false) {
        eprintln!("warning: {err:#}");
    }
}

/// Stop the running install/update/uninstall now, interrupting its current step.
#[tauri::command]
fn force_stop() {
    engine::force_stop();
    if let Err(err) = furnish::service::stop(true) {
        eprintln!("warning: {err:#}");
    }
}

/// How often the load sampler measures; FURNISH_LOAD_SAMPLE_SECONDS overrides the default 10 s
/// (the window is 30 samples, so 10 s means 5 minutes).
fn load_interval() -> std::time::Duration {
    let secs = std::env::var("FURNISH_LOAD_SAMPLE_SECONDS").ok().and_then(|s| s.parse().ok()).unwrap_or(10);
    std::time::Duration::from_secs(secs)
}

/// Current system load, and whether automatic updates should wait.
#[tauri::command]
fn system_load() -> furnish::load::Load {
    furnish::load::current(&furnish::settings::load().load_limits, load_interval())
}

/// Settings in effect, which ones the admin policy locks, and whether users may add collections.
#[tauri::command]
fn get_settings() -> furnish::settings::Effective {
    furnish::settings::effective()
}

/// Change one setting for this user (refused for settings the admin policy locks).
#[tauri::command]
fn set_setting(key: String, value: serde_json::Value) -> CmdResult<()> {
    furnish::settings::set(&key, value).map_err(|e| format!("{e:#}"))?;
    if key == "startAtSignIn" {
        autostart::apply(furnish::settings::load().start_at_sign_in).map_err(|e| format!("{e:#}"))?;
    }
    Ok(())
}

/// The tray menu's texts in the app's language, and what furnish is doing.
#[tauri::command]
fn set_tray(app: tauri::AppHandle, texts: tray::Texts) -> CmdResult<()> {
    tray::update(&app, &texts).map_err(|e| e.to_string())
}

/// Clone the admin's preset collections this user doesn't have yet; returns problems.
#[tauri::command]
async fn ensure_presets() -> CmdResult<Vec<String>> {
    blocking(|| Ok(catalog::ensure_presets())).await
}

/// Fetch the organizations' descriptions again and add the collections new in those of addresses
/// the user added. Returns the added collections and problems: descriptions that fail the checks
/// (the saved ones stay), collections that couldn't be added. Endpoints that don't answer keep
/// theirs without a problem.
#[tauri::command]
async fn refresh_endpoints() -> CmdResult<furnish::endpoint::Refresh> {
    blocking(|| Ok(furnish::endpoint::refresh())).await
}

#[tauri::command]
async fn list_collections() -> CmdResult<Vec<CollectionInfo>> {
    blocking(|| catalog::collections()?.iter().map(collection_info).collect()).await
}

/// Tell the UI the sign-in page's URL (in case the browser doesn't open) and which address it is for.
fn sign_in_event(app: &tauri::AppHandle, origin: &str) -> impl Fn(&str) {
    let (app, origin) = (app.clone(), origin.to_string());
    move |url| {
        let _ = app.emit("furnish://sign-in", SignInStarted { url: url.to_string(), origin: origin.clone() });
    }
}

#[derive(Serialize, Clone)]
struct SignInStarted {
    url: String,
    origin: String,
}

#[tauri::command]
fn is_furnish_address(input: String) -> bool {
    furnish::endpoint::origin(&input).is_some()
}

/// Add a git collection, or an organization's furnish address (`furnish.myorg.tld`): then sign
/// in in the browser (its URL comes as a `furnish://sign-in` event) and add all its collections.
#[tauri::command]
async fn add_collection(
    app: tauri::AppHandle,
    url: String,
    name: Option<String>,
    username: Option<String>,
    password: Option<String>,
    tracking: Option<furnish::settings::Tracking>,
) -> CmdResult<Vec<CollectionInfo>> {
    blocking(move || {
        if let Some(origin) = furnish::endpoint::origin(&url) {
            anyhow::ensure!(tracking.is_none(), "this furnish address supplies the update source for each of its collections");
            let added = catalog::add_endpoint(&origin, sign_in_event(&app, &origin))?;
            return added.iter().map(collection_info).collect();
        }
        let name = name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| catalog::name_from_url(&url));
        let credentials = username.as_deref().filter(|u| !u.is_empty()).zip(password.as_deref());
        Ok(vec![collection_info(&catalog::add_with_tracking(url.trim(), name.trim(), credentials, tracking.as_ref())?)?])
    })
    .await
}

/// A furnish link (`furnish://add?endpoint=…`): the furnish address it names and what adding it
/// would do, for the user to confirm. Anything but a furnish address (git URLs, paths) is refused.
#[tauri::command]
async fn link_preview(link: String) -> CmdResult<furnish::endpoint::Preview> {
    blocking(move || {
        let origin = furnish::endpoint::link_origin(&link).ok_or_else(|| anyhow::anyhow!("the link doesn't name a furnish address"))?;
        furnish::endpoint::preview(&origin)
    })
    .await
}

/// Add a furnish address the user confirmed (see `link_preview`): sign in, add its collections.
#[tauri::command]
async fn add_endpoint(app: tauri::AppHandle, origin: String) -> CmdResult<Vec<CollectionInfo>> {
    blocking(move || {
        let origin = valid_origin(&origin)?;
        let added = catalog::add_endpoint(&origin, sign_in_event(&app, &origin))?;
        added.iter().map(collection_info).collect()
    })
    .await
}

/// An origin exactly as `endpoint::origin` returns it (`https://furnish.myorg.tld`).
fn valid_origin(origin: &str) -> anyhow::Result<String> {
    furnish::endpoint::origin(origin).filter(|o| o == origin).ok_or_else(|| anyhow::anyhow!("'{origin}' isn't a furnish address"))
}

/// An organization's furnish address and whether the user is signed in to it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EndpointInfo {
    origin: String,
    name: String,
    signed_in: bool,
}

#[tauri::command]
async fn endpoints() -> CmdResult<Vec<EndpointInfo>> {
    blocking(|| {
        Ok(furnish::endpoint::known()
            .into_iter()
            .map(|(origin, d)| EndpointInfo { signed_in: furnish::endpoint::token(&origin, &d).is_ok(), name: d.name, origin })
            .collect())
    })
    .await
}

/// Sign in to an endpoint again, then set up its collections (e.g. presets that needed it).
#[tauri::command]
async fn sign_in(app: tauri::AppHandle, origin: String) -> CmdResult<Vec<String>> {
    blocking(move || {
        let origin = valid_origin(&origin)?;
        let description = furnish::endpoint::discover(&origin)?;
        furnish::endpoint::login(&origin, &description, sign_in_event(&app, &origin))?;
        Ok(catalog::ensure_presets())
    })
    .await
}

/// Apps to install on first launch (policy or endpoint `autoInstall`) not installed that way yet.
#[tauri::command]
fn auto_install_pending() -> Vec<String> {
    furnish::endpoint::auto_install_pending()
}

#[tauri::command]
fn auto_install_done(app: String) -> CmdResult<()> {
    furnish::endpoint::auto_install_done(&app).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn preview_collection_update(name: String) -> CmdResult<catalog::CollectionPreview> {
    blocking(move || catalog::get(&name)?.preview_update(None)).await
}

#[tauri::command]
async fn update_collection(name: String, reviewed: Option<catalog::ReviewedRevisions>) -> CmdResult<Revisions> {
    blocking(move || {
        let updated = catalog::get(&name)?.update_reviewed(reviewed.as_ref());
        std::thread::spawn(furnish::reports::send);
        let (old, new) = updated?;
        Ok(Revisions { old, new })
    })
    .await
}

/// Ask each collection's server for its newest commit without applying it (for the "update
/// available" marks). Offline or failing: nothing changes and nothing is reported.
#[tauri::command]
async fn check_collection_servers() -> CmdResult<()> {
    blocking(|| {
        for collection in catalog::collections()? {
            let _ = collection.check_server();
        }
        Ok(())
    })
    .await
}

/// The folder furnish keeps its collections, settings and logs in.
#[tauri::command]
fn data_dir() -> String {
    catalog::data_dir().display().to_string()
}

/// Start in the current folder, or its closest existing parent for a new destination.
#[tauri::command]
async fn choose_folder(window: tauri::Window, title: String, current: Option<std::path::PathBuf>) -> CmdResult<Option<String>> {
    let directory = blocking(move || Ok(current.as_deref().filter(|p| p.is_absolute())
        .and_then(|p| p.ancestors().find(|p| p.is_dir())).map(Path::to_owned))).await?;
    #[cfg(target_os = "linux")]
    let selected = glib::MainContext::default().spawn_from_within(move || async move {
        use gtk::prelude::*;
        let parent = window.gtk_window().map_err(|e| e.to_string())?;
        let picker = gtk::FileChooserNative::new(Some(&title), Some(&parent), gtk::FileChooserAction::SelectFolder, None, None);
        picker.set_modal(true);
        if let Some(path) = directory {
            picker.set_current_folder(path);
        }
        Ok::<_, String>(if picker.run_future().await == gtk::ResponseType::Accept { picker.filename() } else { None })
    }).await.map_err(|e| e.to_string())??;
    #[cfg(not(target_os = "linux"))]
    let selected = {
        let mut picker = rfd::AsyncFileDialog::new().set_title(title).set_parent(&window).set_can_create_directories(true);
        if let Some(path) = directory {
            picker = picker.set_directory(path);
        }
        picker.pick_folder().await.map(|file| file.path().to_owned())
    };
    selected.map(|path| path.into_os_string().into_string()
        .map_err(|_| "The selected folder name is not valid Unicode".to_string())).transpose()
}

/// Ask the desktop's file manager to open an existing directory selected in Settings.
#[tauri::command]
async fn open_directory(path: std::path::PathBuf) -> CmdResult<()> {
    blocking(move || {
        anyhow::ensure!(path.is_absolute() && path.is_dir(), "Folder does not exist: {}", path.display());
        #[cfg(windows)]
        std::process::Command::new("explorer.exe").arg(&path).spawn()?;
        #[cfg(not(windows))]
        {
            let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
            let status = std::process::Command::new(opener).arg(&path).status()?;
            anyhow::ensure!(status.success(), "{opener} could not open {}", path.display());
        }
        Ok(())
    }).await
}

/// How this NixOS computer is set up; `None` on other systems.
#[tauri::command]
fn nixos_setup() -> Option<furnish::nixos::Setup> {
    furnish::nixos::setup()
}

/// Where reports go (the organization's furnish addresses that take them).
#[tauri::command]
fn report_destinations() -> Vec<String> {
    furnish::reports::destination_names()
}

/// The installed apps, for the daily inventory report (only with `reporting: all`, once a day).
#[tauri::command]
fn report_inventory(apps: Vec<(String, String)>) {
    if !furnish::reports::inventory_due() {
        return;
    }
    let apps = apps
        .into_iter()
        .filter_map(|(app, status)| {
            let status = match status.as_str() {
                "installed" => "installed",
                "needsUpdate" => "needsUpdate",
                _ => return None,
            };
            Some(furnish::reports::Installed { app, status })
        })
        .collect();
    furnish::reports::inventory(apps);
    std::thread::spawn(furnish::reports::send);
}

#[tauri::command]
async fn remove_collection(name: String) -> CmdResult<()> {
    blocking(move || catalog::remove(&name)).await
}

fn main() -> std::process::ExitCode {
    if std::env::args_os().skip(1).eq(["--build-info"]) {
        println!("{}", serde_json::to_string(&build_info::current()).unwrap());
        return std::process::ExitCode::SUCCESS;
    }
    // DSC runs furnish's configuration resources as `furnish-gui dsc-resource …` (see engine::init).
    if let Some(code) = furnish::resources::dispatch() {
        return code.into();
    }
    if let Some(resource) = furnish::nixos_apps::ResourceCommand::from_desktop_args() {
        return resource.run();
    }
    desktop_update::initialize();
    // On Wayland the compositor finds the app's icon via the desktop file named like the app ID,
    // which GTK takes from the program name ("furnish-gui", the binary). The packages install
    // furnish.desktop, so name the program to match.
    #[cfg(target_os = "linux")]
    glib::set_prgname(Some("furnish"));

    // Must run before Tauri starts any threads: both set process environment variables.
    // FURNISH_GRAPHICAL makes resources ask for admin rights with the system dialog (pkexec)
    // instead of sudo in whatever terminal the app happened to be started from.
    // SAFETY: still single-threaded here.
    unsafe { std::env::set_var("FURNISH_GRAPHICAL", "1") };
    if let Err(err) = engine::init(&catalog::data_dir().join("resources")) {
        eprintln!("furnish: failed to prepare bundled resources: {err:#}");
        std::process::exit(1);
    }
    furnish::load::start_sampler(load_interval());
    // Tauri's native About menu and updater read PackageInfo as well.
    let mut context = tauri::generate_context!();
    context.package_info_mut().version = build_info::current().version.parse().expect("validated desktop version");
    let exit_code = tauri::Builder::default()
        // Started again while running (from the menu, a furnish:// link): show the window. The
        // link itself reaches the window through the deep-link plugin.
        .plugin(tauri_plugin_single_instance::init(|app, args, _| {
            if !args.iter().any(|a| a == autostart::HIDDEN) {
                tray::show(app);
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            use tauri_plugin_deep_link::DeepLinkExt;
            // Started at sign-in: only the tray icon, the window stays hidden until opened.
            let hidden = std::env::args().any(|a| a == autostart::HIDDEN);
            let brand = branding_info().name;
            let config = app.config().app.windows.iter().find(|w| w.label == "main").cloned().ok_or("no main window in tauri.conf.json")?;
            tauri::WebviewWindowBuilder::from_config(app.handle(), &config)?.title(&brand).visible(!hidden).build()?;
            app_icon::refresh(app.handle(), &brand);
            if let Err(err) = tray::create(app.handle(), &tray::Texts::english(&brand)) {
                eprintln!("warning: no tray icon: {err:#}");
            }
            // The policy may have changed the setting, or furnish moved (AppImage).
            std::thread::spawn(autostart::sync);
            // Bring the confirmation into view, including when the desktop is hidden in the tray.
            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                tray::show(&handle);
                for url in event.urls() {
                    let _ = handle.emit("furnish://link", url.to_string());
                }
            });
            #[cfg(any(target_os = "linux", windows))]
            let _ = app.deep_link().register_all();
            // Stream what installs are doing to the UI's activity log.
            let handle = app.handle().clone();
            engine::capture_logs(move |line| {
                let _ = handle.emit("furnish://log", line);
            });
            Ok(())
        })
        // Closing the window keeps furnish running in the tray. Without a tray it quits, but
        // only after a running install.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::ThemeChanged(_) = event {
                tray::refresh_icon(window.app_handle());
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let app = window.app_handle();
                if tray::shown(app) {
                    let _ = window.hide();
                    let _ = app.emit("furnish://to-tray", ());
                } else {
                    tray::quit(app);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            description_image,
            open_description_link,
            get_branding,
            endpoints,
            sign_in,
            auto_install_pending,
            auto_install_done,
            pending_link,
            list_apps,
            list_kits,
            app_status,
            install,
            install_previous,
            install_current,
            previous_version,
            install_version,
            app_versions,
            set_collection_setting,
            build_info,
            self_update_status,
            desktop_update::desktop_update_status,
            desktop_update::restart_desktop,
            #[cfg(target_os = "macos")]
            cli_link_installed,
            #[cfg(target_os = "macos")]
            install_cli_link,
            check_self_update,
            apply_self_update,
            open_licenses,
            open_directory,
            choose_folder,
            report_destinations,
            nixos_setup,
            report_inventory,
            uninstall,
            reinstall,
            set_option,
            plan,
            abort,
            force_stop,
            get_settings,
            system_load,
            set_setting,
            set_tray,
            ensure_presets,
            refresh_endpoints,
            list_collections,
            is_furnish_address,
            add_collection,
            link_preview,
            add_endpoint,
            update_collection,
            preview_collection_update,
            check_collection_servers,
            data_dir,
            remove_collection
        ])
        .build(context)
        .expect("failed to start the furnish window")
        .run_return(|app, event| match event {
            // Quit from the macOS app menu (Cmd+Q) while installing: after the install.
            tauri::RunEvent::ExitRequested { code: None, api, .. } => {
                api.prevent_exit();
                tray::quit(app);
            }
            // macOS: the Dock icon clicked while the window is hidden.
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { has_visible_windows: false, .. } => tray::show(app),
            _ => {}
        });
    desktop_update::finish(exit_code)
}
