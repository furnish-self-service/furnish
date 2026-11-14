// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! furnish GUI backend: thin Tauri commands over the same library the CLI uses.

use std::{fs, path::Path};

use base64::Engine as _;
use furnish::{
    catalog::{self, App},
    engine::{self, Status},
};
use serde::Serialize;
use tauri::{Emitter, async_runtime::spawn_blocking};

type CmdResult<T> = Result<T, String>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    /// `collection/id`: unique across collections.
    key: String,
    id: String,
    collection: String,
    description: String,
    categories: Vec<String>,
    /// Data URL of the app's icon, if the collection provides one.
    icon: Option<String>,
    resources: Vec<ResourceInfo>,
    /// The DSC configuration document as written in the collection, for experts to inspect.
    source: String,
}

#[derive(Serialize)]
struct ResourceInfo {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    condition: Option<String>,
}

#[derive(Serialize)]
struct CollectionInfo {
    name: String,
    url: String,
    revision: String,
    apps: usize,
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
        .map_err(|e| format!("{e:#}"))
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
        description: app.description.clone(),
        categories: app.categories.clone(),
        icon: app.icon.as_deref().and_then(icon_data_url),
        resources,
        source: fs::read_to_string(&app.path).unwrap_or_default(),
    }
}

fn icon_data_url(path: &Path) -> Option<String> {
    let mime = match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => return None,
    };
    let bytes = fs::read(path).ok().filter(|b| b.len() <= 1024 * 1024)?;
    Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

/// Best effort: a collection whose git metadata can't be read still shows up (as "unknown"),
/// so one broken repository doesn't hide the others.
fn collection_info(c: &catalog::Collection) -> anyhow::Result<CollectionInfo> {
    let unknown = |_| "unknown".to_string();
    Ok(CollectionInfo {
        name: c.name.clone(),
        url: c.url().unwrap_or_else(unknown),
        revision: c.revision().unwrap_or_else(unknown),
        apps: c.apps().len(),
    })
}

#[tauri::command]
async fn list_apps() -> CmdResult<Vec<AppInfo>> {
    blocking(|| Ok(catalog::all_apps()?.iter().map(app_info).collect())).await
}

#[tauri::command]
async fn app_status(key: String) -> CmdResult<Status> {
    blocking(move || engine::status(&catalog::find_app(&key)?.doc)).await
}

/// A step progress event for the UI, tagged with the app it belongs to.
#[derive(Clone, Serialize)]
struct ProgressEvent {
    key: String,
    #[serde(flatten)]
    progress: engine::Progress,
}

/// Run an install/update (or uninstall), streaming step progress as `furnish://progress` events.
async fn run(app: tauri::AppHandle, key: String, uninstall: bool) -> CmdResult<()> {
    blocking(move || {
        let doc = catalog::find_app(&key)?.doc;
        engine::run(&doc, uninstall, |progress| {
            let _ = app.emit("furnish://progress", ProgressEvent { key: key.clone(), progress });
        })
    })
    .await
}

#[tauri::command]
async fn install(app: tauri::AppHandle, key: String) -> CmdResult<()> {
    run(app, key, false).await
}

#[tauri::command]
async fn uninstall(app: tauri::AppHandle, key: String) -> CmdResult<()> {
    run(app, key, true).await
}

/// Preview installing (or uninstalling) without changing anything (DSC what-if).
#[tauri::command]
async fn plan(key: String, uninstall: bool) -> CmdResult<Vec<engine::Step>> {
    blocking(move || engine::plan(&catalog::find_app(&key)?.doc, uninstall)).await
}

#[tauri::command]
async fn list_collections() -> CmdResult<Vec<CollectionInfo>> {
    blocking(|| catalog::collections()?.iter().map(collection_info).collect()).await
}

#[tauri::command]
async fn add_collection(url: String, name: Option<String>) -> CmdResult<CollectionInfo> {
    blocking(move || {
        let name = name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| catalog::name_from_url(&url));
        collection_info(&catalog::add(url.trim(), name.trim())?)
    })
    .await
}

#[tauri::command]
async fn update_collection(name: String) -> CmdResult<Revisions> {
    blocking(move || {
        let (old, new) = catalog::get(&name)?.update()?;
        Ok(Revisions { old, new })
    })
    .await
}

#[tauri::command]
async fn remove_collection(name: String) -> CmdResult<()> {
    blocking(move || catalog::remove(&name)).await
}

fn main() {
    // Must run before Tauri starts any threads: both set process environment variables.
    // FURNISH_GRAPHICAL makes resources ask for admin rights with the system dialog (pkexec)
    // instead of sudo in whatever terminal the app happened to be started from.
    // SAFETY: still single-threaded here.
    unsafe { std::env::set_var("FURNISH_GRAPHICAL", "1") };
    if let Err(err) = engine::init(&catalog::data_dir().join("resources")) {
        eprintln!("furnish: failed to prepare bundled resources: {err:#}");
        std::process::exit(1);
    }
    tauri::Builder::default()
        .setup(|app| {
            // Stream what installs are doing to the UI's activity log.
            let handle = app.handle().clone();
            engine::capture_logs(move |line| {
                let _ = handle.emit("furnish://log", line);
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_apps,
            app_status,
            install,
            uninstall,
            plan,
            list_collections,
            add_collection,
            update_collection,
            remove_collection
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the furnish window");
}
