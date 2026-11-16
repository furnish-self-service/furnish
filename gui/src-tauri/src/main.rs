// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! furnish GUI backend: thin Tauri commands over the same library the CLI uses.

mod autostart;
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
    /// Options the app declares, with their current values.
    options: Vec<options::Resolved>,
    /// Installed with other option values than the current ones: a reinstall applies them.
    options_differ: bool,
    /// Recommended for this user (`metadata.furnish.recommended`).
    recommended: bool,
    /// Required collection options nobody answered yet: installing waits for them.
    unanswered: Vec<String>,
    /// Held at this earlier version because the collection's current one failed.
    held: Option<catalog::Version>,
}

#[derive(Serialize)]
struct ResourceInfo {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    condition: Option<String>,
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

#[tauri::command]
fn get_branding() -> BrandingInfo {
    branding_info()
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
    let resources = app.doc["resources"]
        .as_array()
        .map(|list| {
            list.iter()
                .map(|r| ResourceInfo {
                    name: r["name"].as_str().unwrap_or_default().to_string(),
                    kind: r["type"].as_str().unwrap_or_default().to_string(),
                    condition: r["condition"].as_str().map(String::from),
                })
                .collect()
        })
        .unwrap_or_default();
    AppInfo {
        key: format!("{}/{}", app.collection, app.id),
        id: app.id.clone(),
        collection: app.collection.clone(),
        name: app.name.clone(),
        description: app.description.clone(),
        categories: app.categories.clone(),
        icon: app.icon.as_deref().and_then(icon_data_url),
        icon_dark: app.icon_dark.as_deref().and_then(icon_data_url),
        resources,
        source: fs::read_to_string(&app.path).unwrap_or_default(),
        dependencies: items(&app.collection, &app.dependencies_for(&app.current_values())),
        options: app.options(),
        options_differ: app.options_differ(),
        recommended: app.recommended(),
        unanswered: app.unanswered(),
        held: app.held_version(),
    }
}

fn items(collection: &str, items: &[catalog::Item]) -> Vec<ItemInfo> {
    items
        .iter()
        .map(|i| ItemInfo { key: format!("{collection}/{}", i.app), required: i.required, selected: i.required || i.selected })
        .collect()
}

fn icon_data_url(path: &Path) -> Option<String> {
    let mime = match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => return None,
    };
    // Size first: the collection decides what the file is, and reading it all could be costly.
    const LIMIT: u64 = 1024 * 1024;
    if fs::metadata(path).ok()?.len() > LIMIT {
        return None;
    }
    let mut bytes = Vec::new();
    fs::File::open(path).ok()?.take(LIMIT + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > LIMIT {
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
async fn app_status(key: String) -> CmdResult<engine::Check> {
    blocking(move || catalog::find_app(&key)?.check()).await
}

/// A step progress event for the UI, tagged with the app it belongs to.
#[derive(Clone, Serialize)]
struct ProgressEvent {
    key: String,
    #[serde(flatten)]
    progress: engine::Progress,
}

#[derive(Clone, Copy)]
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

/// Run an install/update, uninstall or reinstall, streaming step progress as `furnish://progress` events.
/// `trigger`: who started it, for reports: `update` (automatic updates), `autoInstall`, else the user.
async fn run(handle: tauri::AppHandle, key: String, action: Action, trigger: Option<String>) -> CmdResult<Option<FellBack>> {
    let trigger = match trigger.as_deref() {
        Some("update") => "update",
        Some("autoInstall") => "autoInstall",
        _ => "user",
    };
    let running = tray::Running::start(&handle).ok_or("furnish is quitting")?;
    let result = blocking(move || furnish::reports::with_trigger(trigger, || {
        let app = catalog::find_app(&key)?;
        let report = |progress| {
            let _ = handle.emit("furnish://progress", ProgressEvent { key: key.clone(), progress });
        };
        match action {
            Action::Install(which) => match app.install_version(which, report) {
                Err(err) if which == catalog::Which::Shown && !err.is::<engine::Cancelled>() && app.held.is_none() => {
                    let automatic = furnish::settings::for_collection(&app.collection).settings.fallback == furnish::settings::Fallback::Automatic;
                    let Some(version) = app.previous_version().ok().filter(|_| automatic) else { return Err(err) };
                    app.install_version(catalog::Which::Previous, report)?;
                    Ok(Some(FellBack { date: version.date, error: format!("{err:#}") }))
                }
                result => result.map(|()| None),
            },
            Action::Uninstall => app.uninstall(report).map(|()| None),
            Action::Reinstall => app.reinstall(report).map(|()| None),
        }
    }))
    .await;
    std::thread::spawn(furnish::reports::send);
    drop(running); // quits now if Quit was chosen while this ran
    result
}

/// Install or update; with the fallback "automatic", a failure installs the previous version.
#[tauri::command]
async fn install(app: tauri::AppHandle, key: String, trigger: Option<String>) -> CmdResult<Option<FellBack>> {
    run(app, key, Action::Install(catalog::Which::Shown), trigger).await
}

/// Install the previous version of an app (see `previous_version`).
#[tauri::command]
async fn install_previous(app: tauri::AppHandle, key: String, trigger: Option<String>) -> CmdResult<Option<FellBack>> {
    run(app, key, Action::Install(catalog::Which::Previous), trigger).await
}

/// For an app held at an earlier version: install the collection's current one.
#[tauri::command]
async fn install_current(app: tauri::AppHandle, key: String, trigger: Option<String>) -> CmdResult<Option<FellBack>> {
    run(app, key, Action::Install(catalog::Which::Current), trigger).await
}

/// The earlier version furnish would fall back to; an error says why there is none.
#[tauri::command]
async fn previous_version(key: String) -> CmdResult<catalog::Version> {
    blocking(move || catalog::find_app(&key)?.previous_version()).await
}

#[tauri::command]
async fn uninstall(app: tauri::AppHandle, key: String, trigger: Option<String>) -> CmdResult<Option<FellBack>> {
    run(app, key, Action::Uninstall, trigger).await
}

/// Uninstall with the options the app was installed with, then install with the current ones.
#[tauri::command]
async fn reinstall(app: tauri::AppHandle, key: String, trigger: Option<String>) -> CmdResult<Option<FellBack>> {
    run(app, key, Action::Reinstall, trigger).await
}

/// furnish's version (builds of main: 0.1.0+alpha.<build>).
#[tauri::command]
fn version() -> &'static str {
    option_env!("FURNISH_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
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

/// Fetch the organizations' descriptions again; returns those that fail the checks (the saved
/// ones stay). Endpoints that don't answer keep theirs without a problem.
#[tauri::command]
async fn refresh_endpoints() -> CmdResult<Vec<String>> {
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

/// Add a git collection, or an organization's furnish address (`furnish.myorg.tld`): then sign
/// in in the browser (its URL comes as a `furnish://sign-in` event) and add all its collections.
#[tauri::command]
async fn add_collection(
    app: tauri::AppHandle,
    url: String,
    name: Option<String>,
    username: Option<String>,
    password: Option<String>,
) -> CmdResult<Vec<CollectionInfo>> {
    blocking(move || {
        if let Some(origin) = furnish::endpoint::origin(&url) {
            let added = catalog::add_endpoint(&origin, sign_in_event(&app, &origin))?;
            return added.iter().map(collection_info).collect();
        }
        let name = name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| catalog::name_from_url(&url));
        let credentials = username.as_deref().filter(|u| !u.is_empty()).zip(password.as_deref());
        Ok(vec![collection_info(&catalog::add(url.trim(), name.trim(), credentials)?)?])
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
async fn update_collection(name: String) -> CmdResult<Revisions> {
    blocking(move || {
        let updated = catalog::get(&name)?.update();
        std::thread::spawn(furnish::reports::send);
        let (old, new) = updated?;
        Ok(Revisions { old, new })
    })
    .await
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

fn main() {
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
    tauri::Builder::default()
        // Started again while running (from the menu, a furnish:// link): show the window. The
        // link itself reaches the window through the deep-link plugin.
        .plugin(tauri_plugin_single_instance::init(|app, args, _| {
            if !args.iter().any(|a| a == autostart::HIDDEN) {
                tray::show(app);
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .setup(|app| {
            use tauri_plugin_deep_link::DeepLinkExt;
            // Started at sign-in: only the tray icon, the window stays hidden until opened.
            let hidden = std::env::args().any(|a| a == autostart::HIDDEN);
            let brand = branding_info().name;
            let config = app.config().app.windows.iter().find(|w| w.label == "main").cloned().ok_or("no main window in tauri.conf.json")?;
            tauri::WebviewWindowBuilder::from_config(app.handle(), &config)?.title(&brand).visible(!hidden).build()?;
            if let Err(err) = tray::create(app.handle(), &tray::Texts::english(&brand)) {
                eprintln!("warning: no tray icon: {err:#}");
            }
            // The policy may have changed the setting, or furnish moved (AppImage).
            std::thread::spawn(autostart::sync);
            // Links (furnish://add?endpoint=…) go to the UI, which asks before adding anything.
            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
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
            set_collection_setting,
            version,
            open_licenses,
            report_destinations,
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
            add_collection,
            link_preview,
            add_endpoint,
            update_collection,
            remove_collection
        ])
        .build(tauri::generate_context!())
        .expect("failed to start the furnish window")
        .run(|app, event| match event {
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
}
