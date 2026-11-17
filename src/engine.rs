//! Thin wrapper around the embedded DSC engine. The only module that touches dsc-lib,
//! so upstream API churn stays contained here.

use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::Result;
use dsc_lib::{
    configure::{Configurator, config_doc::ExecutionKind, config_result::ResourceTestResult},
    discovery::{Discovery, command_discovery::ImportedManifest, discovery_trait::{DiscoveryFilter, DiscoveryKind}},
    dscerror::DscError,
    dscresources::{
        dscresource::Capability,
        invoke_result::{GetResult, SetResult, TestResult},
    },
    progress::ProgressFormat,
    types::{FullyQualifiedTypeName, TypeNameFilter},
};
use serde_json::Value;

use crate::required::Decision;

/// Resources shipped inside the furnish binary: (file name, contents).
/// Same resource types on every OS, with OS-specific implementations.
#[cfg(not(windows))]
const BUNDLED_RESOURCES: &[(&str, &str)] = &[
    ("furnish-common.sh", include_str!("../resources/unix/furnish-common.sh")),
    ("furnish-homebrew.sh", include_str!("../resources/unix/furnish-homebrew.sh")),
    ("furnish-nix.sh", include_str!("../resources/unix/furnish-nix.sh")),
    ("furnish-package.dsc.resource.json", include_str!("../resources/unix/furnish-package.dsc.resource.json")),
    ("furnish-package.dsc.resource.sh", include_str!("../resources/unix/furnish-package.dsc.resource.sh")),
    ("furnish-installer.dsc.resource.json", include_str!("../resources/unix/furnish-installer.dsc.resource.json")),
    ("furnish-installer.dsc.resource.sh", include_str!("../resources/unix/furnish-installer.dsc.resource.sh")),
    ("furnish-nixos.dsc.resource.json", include_str!("../resources/unix/furnish-nixos.dsc.resource.json")),
    ("furnish-nixos.dsc.resource.sh", include_str!("../resources/unix/furnish-nixos.dsc.resource.sh")),
];
#[cfg(windows)]
const BUNDLED_RESOURCES: &[(&str, &str)] = &[
    (
        "furnish-package.dsc.resource.json",
        include_str!("../resources/windows/furnish-package.dsc.resource.json"),
    ),
    (
        "furnish-package.dsc.resource.ps1",
        include_str!("../resources/windows/furnish-package.dsc.resource.ps1"),
    ),
    ("furnish-installer.dsc.resource.json", include_str!("../resources/windows/furnish-installer.dsc.resource.json")),
    ("furnish-installer.dsc.resource.ps1", include_str!("../resources/windows/furnish-installer.dsc.resource.ps1")),
];

/// dsc-lib mutates the process environment (PATH) during resource discovery, so engine
/// calls must not overlap. The GUI runs them on worker threads.
// ponytail: one global lock; installs run one at a time, which is also what package managers want.
static ENGINE: Mutex<()> = Mutex::new(());

/// Set by `abort`: skip the remaining steps of the running operation.
static ABORT: AtomicBool = AtomicBool::new(false);
/// File whose existence asks the running step to stop (see `force_stop`). Resources get its path
/// as FURNISH_STOP_FILE; the watcher runs with the step's privileges, so it may signal root processes.
static STOP_FILE: OnceLock<PathBuf> = OnceLock::new();

/// Message of the error an operation ends with when the user cancelled it.
pub const CANCELLED: &str = "cancelled";

/// The operation was cancelled by the user: an abort, a force stop, or a refused password dialog.
#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(CANCELLED)
    }
}

impl std::error::Error for Cancelled {}

/// Stop the running operation after its current step.
pub fn abort() {
    ABORT.store(true, Ordering::SeqCst);
}

/// Stop the running operation now: abort, and interrupt the current step (SIGTERM to its
/// command). Steps whose resource doesn't support it still finish.
pub fn force_stop() {
    abort();
    if let Some(file) = STOP_FILE.get() {
        let _ = fs::write(file, "");
    }
}

/// Resources report a user cancellation with these exit code descriptions (see the manifests).
fn is_cancellation(error: &str) -> bool {
    error.contains("Cancelled: administrator rights were not granted") || error.contains("Cancelled: stopped by the user")
}

/// One line of what an operation is doing, for experts.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LogLine {
    /// `error`, `warn`, `info`, `debug` or `output` (a line a resource process wrote, e.g. pacman).
    pub level: &'static str,
    pub message: String,
}

/// Send DSC's progress and every line resource processes print to `sink`.
/// Call once at startup; later calls are ignored.
pub fn capture_logs(sink: impl Fn(LogLine) + Send + Sync + 'static) {
    use tracing_subscriber::prelude::*;
    if LOG_SINK.set(Box::new(sink)).is_ok() {
        let _ = tracing_subscriber::registry().with(LogForwarder).try_init();
    }
}

static LOG_SINK: OnceLock<Box<dyn Fn(LogLine) + Send + Sync>> = OnceLock::new();

/// Pass on a log line from elsewhere (the furnish service) as if DSC had logged it here.
pub fn emit_log(level: &str, message: String) {
    let level = ["error", "warn", "info", "debug", "output"].into_iter().find(|l| *l == level).unwrap_or("info");
    if let Some(sink) = LOG_SINK.get() {
        sink(LogLine { level, message });
    }
}

struct LogForwarder;

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for LogForwarder {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        let meta = event.metadata();
        if !meta.target().starts_with("dsc_lib") {
            return;
        }
        let mut message = String::new();
        event.record(&mut MessageVisitor(&mut message));
        if let (Some(line), Some(sink)) = (log_line(*meta.level(), &message), LOG_SINK.get()) {
            sink(line);
        }
    }
}

struct MessageVisitor<'a>(&'a mut String);

impl tracing::field::Visit for MessageVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            *self.0 = format!("{value:?}");
        }
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            *self.0 = value.to_string();
        }
    }
}

/// Decide what an expert sees: resource output ("PID n: ...") at any level, and DSC's own
/// info/warn/error messages. DSC's internal debug/trace chatter (schemas, discovery) is dropped.
fn log_line(level: tracing::Level, message: &str) -> Option<LogLine> {
    use tracing::Level;
    let from_resource = message
        .strip_prefix("PID ")
        .and_then(|rest| rest.split_once(": "))
        .filter(|(pid, _)| pid.chars().all(|c| c.is_ascii_digit()))
        .map(|(_, text)| text);
    let (level, message) = match (from_resource, level) {
        (Some(text), Level::ERROR) => ("error", text),
        (Some(text), Level::WARN) => ("warn", text),
        (Some(text), Level::INFO) => ("info", text),
        (Some(text), _) => ("output", text),
        (None, Level::ERROR) => ("error", message),
        (None, Level::WARN) => ("warn", message),
        (None, Level::INFO) => ("info", message),
        (None, _) => return None,
    };
    let message = message.trim();
    (!message.is_empty()).then(|| LogLine { level, message: message.to_string() })
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Installed,
    NotInstalled,
    /// Present, but not in the desired state (e.g. outdated or modified).
    NeedsUpdate,
    /// Every resource's `condition` excluded it on this machine (e.g. a Windows-only app on Linux).
    Unavailable,
    /// A tool a resource needs is missing, e.g. Homebrew; see [`Check::reason`].
    Blocked,
}

/// An app's status, with what it needs if it is blocked.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Check {
    pub status: Status,
    /// For `Blocked`: what is missing, e.g. "Homebrew (brew)".
    pub reason: Option<String>,
    /// furnish can uninstall it here (see [`unremovable`]); not if it is [`Check::external`].
    pub removable: bool,
    /// Installed outside furnish (see [`External`]).
    pub external: Option<External>,
    /// A required version isn't met here (see [`crate::required`]).
    pub required: Option<crate::required::Required>,
}

/// A package installed outside furnish. furnish shows it as installed and leaves it to what
/// installed it: it doesn't install it again, remove it or update it. Either the system's own
/// configuration installs it, NixOS (`environment.systemPackages`, `users.users.<name>.packages`)
/// or Home Manager, or its program is there although the package manager doesn't have the package
/// (Furnish/Package's `command`), e.g. installed with npm or the vendor's installer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct External {
    /// `nixos`, `home-manager` or `other` (the resource's `managedBy`).
    pub by: String,
    /// The nixpkgs attribute, e.g. `jq` or `kdePackages.kdenlive`; for `other` the package's name.
    pub package: String,
    /// The configuration's folder, if known (the resource's `declaredIn`).
    pub configuration: Option<String>,
    /// For `other`: where the program is, e.g. `/opt/homebrew/bin/claude` (the resource's `path`).
    pub path: Option<String>,
}

impl External {
    /// Where it comes from, for the list: "in your NixOS configuration (/home/alex/cfg)",
    /// "installed outside furnish: /opt/homebrew/bin/claude".
    pub fn summary(&self) -> String {
        match self.configuration_name() {
            Some(what) => format!("in {}", self.within(what)),
            None => format!("installed outside furnish: {}", self.path.as_deref().unwrap_or(&self.package)),
        }
    }

    /// Why furnish doesn't install it: "it is in your NixOS configuration".
    pub fn reason(&self) -> String {
        match self.configuration_name() {
            Some(what) => format!("it is in {}", self.within(what)),
            None => format!("it was installed outside furnish{}", self.at()),
        }
    }

    /// Why furnish doesn't uninstall app `id`, and what to do instead.
    pub fn refusal(&self, id: &str) -> String {
        match self.configuration_name() {
            Some(what) => format!("{id} is in {}: remove {} there", self.within(what), self.package),
            None => format!("{id} was installed outside furnish{}: uninstall it the way it was installed", self.at()),
        }
    }

    fn configuration_name(&self) -> Option<&'static str> {
        match self.by.as_str() {
            "nixos" => Some("your NixOS configuration"),
            "home-manager" => Some("your Home Manager configuration"),
            _ => None,
        }
    }

    fn within(&self, what: &str) -> String {
        match &self.configuration {
            Some(c) => format!("{what} ({c})"),
            None => what.to_string(),
        }
    }

    fn at(&self) -> String {
        self.path.as_ref().map(|p| format!(" ({p})")).unwrap_or_default()
    }
}

/// The first resource whose package was installed outside furnish.
fn external(states: &[Tested]) -> Option<External> {
    states.iter().find_map(|t| {
        let by = t.actual["managedBy"].as_str().filter(|b| matches!(*b, "nixos" | "home-manager" | "other"))?;
        // DSC's desired state holds the resource's properties; the Nix name only if Nix installed it.
        let nix_name = if by == "other" { &Value::Null } else { &t.desired["nixName"] };
        let package = [nix_name, &t.desired["name"], &t.actual["name"]]
            .into_iter()
            .find_map(|v| v.as_str().filter(|s| !s.is_empty()))?;
        let text = |key: &str| t.actual[key].as_str().map(String::from);
        Some(External { by: by.to_string(), package: package.to_string(), configuration: text("declaredIn"), path: text("path") })
    })
}

/// Resources report a missing tool as a plain stderr line "furnish: needs <what>" (exit 11).
pub fn missing_reason(error: &str) -> Option<String> {
    let rest = &error[error.find("furnish: needs ")? + "furnish: needs ".len()..];
    Some(rest.lines().next().unwrap_or_default().trim().to_string())
}

/// Where furnish wrote its own resources (see `init`).
static BUNDLED_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Write the bundled resources to `dir` and point DSC's resource discovery at them, then at DSC's
/// own resources that furnish ships (`bundled_dsc_dir`), then at the folders DSC normally searches
/// (see `search_dirs`).
/// Must run before any other thread starts, since it mutates the process environment.
pub fn init(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)?;
    for (file, contents) in BUNDLED_RESOURCES {
        let path = dir.join(file);
        fs::write(&path, contents)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
        }
    }
    // The configuration resources run in this program (see resources::dispatch).
    for (file, contents) in crate::resources::manifests(&env::current_exe()?)? {
        fs::write(dir.join(file), contents)?;
    }
    let _ = BUNDLED_DIR.set(dir.to_path_buf());
    let path: Vec<PathBuf> = env::split_paths(&env::var_os("PATH").unwrap_or_default()).collect();
    let in_service = crate::service::in_service();
    let dsc = bundled_dsc_dir().filter(|d| !in_service || crate::service::admin_only(d));
    let others = search_dirs(&path, in_service, crate::service::admin_only);
    let (programs, resources) = discovery_dirs(dir, dsc.as_deref(), others);
    let stop_file = STOP_FILE.get_or_init(|| dir.join(".stop"));
    // SAFETY: called once at startup from main, before any threads are spawned.
    unsafe {
        // DSC searches DSC_RESOURCE_PATH instead of PATH, and finds resources' programs on PATH,
        // then in the resource's folder. furnish's own resources first, then DSC's.
        env::set_var("DSC_RESOURCE_PATH", env::join_paths(resources)?);
        env::set_var("PATH", env::join_paths(programs)?);
        env::set_var("FURNISH_STOP_FILE", stop_file);
        // Resources keep what is slow to find out here, e.g. the names nixpkgs gives packages.
        env::set_var("FURNISH_CACHE_DIR", dir.parent().unwrap_or(dir).join("cache"));
        // Furnish/NixOS runs furnish itself (`nixos-resource`); only the service may rebuild.
        if let Ok(exe) = env::current_exe() {
            env::set_var("FURNISH_EXE", exe);
        }
        env::set_var("FURNISH_SERVICE", if in_service { "1" } else { "" });
        // The settings in effect and what furnish detected (FURNISH_CONFIG__*, FURNISH_DETECTED__*).
        crate::resource_env::init();
    }
    Ok(())
}

/// The folder with DSC's own resources that furnish's installers ship (scripts/fetch-dsc.mjs):
/// beside the program (Windows, and builds in target/), in ../lib/furnish (Linux packages, Nix)
/// or in ../Resources (the macOS app). `None` for a build without it: then only furnish's own
/// resources and those installed on the computer are there.
fn bundled_dsc_dir() -> Option<PathBuf> {
    let exe = env::current_exe().ok()?;
    // The real folder of the program, not of a link to it; Windows keeps plain paths for PATH.
    let exe = if cfg!(windows) { exe } else { exe.canonicalize().ok()? };
    let bin = exe.parent()?;
    let prefix = bin.parent()?;
    let program = if cfg!(windows) { "dsc.exe" } else { "dsc" };
    [bin.join("dsc"), prefix.join("lib").join("furnish").join("dsc"), prefix.join("Resources").join("dsc")]
        .into_iter()
        .find(|d| d.join(program).is_file())
}

/// The folders for finding programs (PATH) and for finding resources (DSC_RESOURCE_PATH): furnish's
/// own resources, DSC's own (`dsc`), then `others`. DSC uses the manifest with the highest version
/// of a type (the first one found if versions are equal), so a folder elsewhere that declares a
/// type of DSC's own resources isn't searched for resources: an older or newer DSC installed on the
/// computer, or a manifest that pretends to be Microsoft.DSC/Group. Its programs stay on PATH,
/// behind furnish's folders.
fn discovery_dirs(own: &Path, dsc: Option<&Path>, others: Vec<PathBuf>) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let first: Vec<PathBuf> = std::iter::once(own.to_path_buf()).chain(dsc.map(Path::to_path_buf)).collect();
    let others: Vec<PathBuf> = others.into_iter().filter(|d| !first.contains(d)).collect();
    let reserved = dsc.map(declared_types).unwrap_or_default();
    let mut resources = first.clone();
    for d in &others {
        match declared_types(d).into_iter().find(|t| reserved.contains(t)) {
            Some(kind) => emit_log(
                "warn",
                format!("Not searching {} for DSC resources: it declares {kind}, which furnish provides", d.display()),
            ),
            None => resources.push(d.clone()),
        }
    }
    let mut programs = first;
    programs.extend(others);
    (programs, resources)
}

/// The resource and extension types the manifests in `dir` declare (lowercase), whatever their
/// conditions, as DSC finds them: files with DSC's manifest extensions, not in subfolders.
fn declared_types(dir: &Path) -> std::collections::HashSet<String> {
    use dsc_lib::discovery::command_discovery::{
        DSC_ADAPTED_RESOURCE_EXTENSIONS, DSC_EXTENSION_EXTENSIONS, DSC_MANIFEST_LIST_EXTENSIONS, DSC_RESOURCE_EXTENSIONS,
    };
    let extensions = [DSC_RESOURCE_EXTENSIONS, DSC_MANIFEST_LIST_EXTENSIONS, DSC_ADAPTED_RESOURCE_EXTENSIONS, DSC_EXTENSION_EXTENSIONS];
    let is_manifest = |name: &str| {
        let name = name.to_lowercase();
        extensions.iter().flatten().any(|ext| name.ends_with(ext))
    };
    let mut types = std::collections::HashSet::new();
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        if !entry.file_name().to_str().is_some_and(is_manifest) {
            continue;
        }
        // YAML reads JSON too.
        let Ok(manifest) = fs::read_to_string(entry.path()).map_err(anyhow::Error::from).and_then(|t| Ok(serde_yaml::from_str::<Value>(&t)?))
        else {
            continue;
        };
        let listed = ["resources", "adaptedResources", "extensions"].into_iter().flat_map(|k| manifest[k].as_array().cloned().unwrap_or_default());
        for m in std::iter::once(manifest.clone()).chain(listed) {
            if let Some(kind) = m["type"].as_str() {
                types.insert(kind.to_lowercase());
            }
        }
    }
    types
}

/// The folders DSC searches for resources besides furnish's own: PATH, as DSC does, and where DSC
/// itself is installed, since its built-in resources (Microsoft.Windows/Registry,
/// RunCommandOnSet, the PowerShell adapters, ...) ship next to the `dsc` program.
///
/// In the furnish service (root or SYSTEM), only folders that just administrators can change
/// (`admin_only`): the folder and every folder above it belong to root (Windows: SYSTEM,
/// Administrators or TrustedInstaller) and nobody else may write to it. A resource is a program
/// the service runs with full rights, so a resource from a folder a user can write, such as
/// /opt/homebrew/bin or a user's own PATH entry, would let that user run anything as root. This
/// rule beats a fixed list of system folders: it keeps working folders that admins add to the
/// system PATH (for example PowerShell's), and it drops system folders an installer left
/// writable. The service also finds programs on this filtered PATH only, since DSC looks up a
/// resource's program on PATH before its own folder.
fn search_dirs(path: &[PathBuf], in_service: bool, admin_only: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = path.to_vec();
    dirs.extend(dsc_install_dirs(path));
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|d| !d.as_os_str().is_empty() && seen.insert(d.clone()));
    if in_service {
        dirs.retain(|d| admin_only(d));
    }
    dirs
}

/// Where DSC is installed: the real folder of `dsc` on PATH (the Linux packages link
/// /usr/bin/dsc to /opt/dsc/dsc), its Linux packages' folder, and on Windows its MSIX package.
fn dsc_install_dirs(path: &[PathBuf]) -> Vec<PathBuf> {
    let program = if cfg!(windows) { "dsc.exe" } else { "dsc" };
    let mut dirs: Vec<PathBuf> = path
        .iter()
        .map(|d| d.join(program))
        .find(|p| p.is_file())
        .and_then(|p| p.canonicalize().ok())
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .into_iter()
        .collect();
    if cfg!(target_os = "linux") {
        dirs.push(PathBuf::from("/opt/dsc"));
    }
    // ponytail: users can't list WindowsApps, so for them the MSIX install counts only if its
    // folder is on PATH; the service (SYSTEM) finds it here. Resolving the `dsc` app execution
    // alias would find it for users too.
    if cfg!(windows) {
        let apps = PathBuf::from(env::var_os("ProgramFiles").unwrap_or_else(|| r"C:\Program Files".into())).join("WindowsApps");
        let mut packages: Vec<PathBuf> = fs::read_dir(apps)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("Microsoft.DesiredStateConfiguration")))
            .collect();
        packages.sort();
        dirs.extend(packages.into_iter().rev());
    }
    dirs
}

/// furnish's resource types must come from furnish: DSC uses the manifest with the highest
/// version of a type from all folders it searches (the first one found if versions are equal),
/// so a manifest elsewhere that declares Furnish/Package could take its place. Checked once, on
/// DSC's first full discovery, which DSC then keeps for the life of the process.
fn ensure_own_resources() -> Result<()> {
    static CHECKED: OnceLock<Result<(), String>> = OnceLock::new();
    let Some(bundled) = BUNDLED_DIR.get() else { return Ok(()) };
    CHECKED
        .get_or_init(|| {
            // Unfiltered, so that DSC's cache holds every resource type afterwards.
            let found = Discovery::new().list_available(&DiscoveryKind::Resource, &TypeNameFilter::default(), None, ProgressFormat::None);
            let manifests: Vec<(String, PathBuf)> = found
                .into_iter()
                .filter_map(|m| match m {
                    ImportedManifest::Resource(r) => Some((r.type_name.to_string(), r.path)),
                    ImportedManifest::Extension(_) => None,
                })
                .collect();
            match foreign_furnish_manifests(&manifests, bundled).first() {
                Some((kind, path)) => Err(format!(
                    "{} declares the resource type {kind}, which only furnish provides. Remove it, or remove its folder from PATH",
                    path.display()
                )),
                None => Ok(()),
            }
        })
        .clone()
        .map_err(anyhow::Error::msg)
}

/// Manifests (type, path) of furnish's resource types that aren't furnish's own, in `bundled`.
fn foreign_furnish_manifests(manifests: &[(String, PathBuf)], bundled: &Path) -> Vec<(String, PathBuf)> {
    let bundled = bundled.canonicalize().unwrap_or_else(|_| bundled.to_path_buf());
    manifests
        .iter()
        .filter(|(kind, _)| crate::catalog::RESOURCE_TYPES.iter().any(|t| t.eq_ignore_ascii_case(kind)))
        .filter(|(_, path)| {
            let dir = path.parent().map(|d| d.canonicalize().unwrap_or_else(|_| d.to_path_buf()));
            dir.as_deref() != Some(bundled.as_path())
        })
        .cloned()
        .collect()
}

/// Whether DSC finds a resource of this type on this computer.
pub fn available(kind: &str) -> Result<bool> {
    let _guard = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    ensure_own_resources()?;
    let Ok(kind) = kind.parse::<FullyQualifiedTypeName>() else { return Ok(false) };
    let filter = DiscoveryFilter::new(&kind, None, None);
    let mut discovery = Discovery::new();
    discovery.find_resources(std::slice::from_ref(&filter), ProgressFormat::None)?;
    Ok(discovery.find_resource(&filter)?.is_some())
}

// DSC v3.3 reports resource failures as `Err`; `had_errors`/`messages` are never set.
pub fn status(doc: &Value) -> Result<Status> {
    Ok(check(doc)?.status)
}

/// Status plus, for a blocked app, what it needs.
pub fn check(doc: &Value) -> Result<Check> {
    let _guard = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    let _working = crate::load::activity(); // load-aware updates ignore what furnish causes
    let removable = unremovable_unlocked(doc)?.is_none();
    let Some(doc) = applicable(doc) else {
        return Ok(Check { status: Status::Unavailable, reason: None, removable, external: None, required: None });
    };
    let mut tested = match test(&doc) {
        Ok(states) => states,
        Err(e) => {
            let reason = missing_resource(&e).or_else(|| missing_reason(&format!("{e:#}"))).ok_or(e)?;
            return Ok(Check { status: Status::Blocked, reason: Some(reason), removable, external: None, required: None });
        }
    };
    let external = external(&tested);
    let (mut required, mut blocked) = (None::<crate::required::Required>, None);
    for t in &mut tested {
        let Some(judged) = &t.judged else { continue };
        let installed = exists(&t.actual);
        match &judged.decision {
            // Held at (or going to) one version: up to date when it is installed.
            Decision::Exactly(v) if installed => t.ok = judged.installed.as_deref() == Some(v.as_str()),
            // Nothing acceptable to install: the app shows what it needs, without an update to offer.
            Decision::NoneOffered if installed => t.ok = true,
            Decision::NoneOffered => blocked = judged.shown.as_ref().map(|r| r.summary.clone()),
            _ => {}
        }
        if let Some(shown) = &judged.shown {
            // An acceptable version is offered: a required update.
            if !shown.acceptable && shown.target.is_some() {
                t.ok = false;
            }
            if required.as_ref().is_none_or(|r| r.acceptable && !shown.acceptable) {
                required = Some(shown.clone());
            }
        }
    }
    let removable = removable && external.is_none();
    if let Some(reason) = blocked {
        return Ok(Check { status: Status::Blocked, reason: Some(reason), removable, external, required });
    }
    let states: Vec<(bool, Value)> = tested.into_iter().map(|t| (t.ok, t.actual)).collect();
    Ok(Check { status: classify(&states), reason: None, removable, external, required })
}

/// The resources (those that apply here) that aren't in their desired state, e.g. "'profile'
/// (Microsoft.Filesystem.File/Content)". The same test as [`check`], which runs anyway when the
/// app's status is shown; furnish runs it right after installing to see the install worked.
pub fn differing(doc: &Value) -> Result<Vec<String>> {
    let _guard = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    let _working = crate::load::activity();
    let Some(doc) = applicable(doc) else { return Ok(vec![]) };
    Ok(test(&doc)?.into_iter().filter(|t| !t.installed_as_required()).map(|t| format!("'{}' ({})", t.name, t.kind)).collect())
}

/// DSC's test of every resource of the document, with the required versions of those that have
/// them judged (see [`crate::required`]). The caller holds the engine lock.
fn test(doc: &Value) -> Result<Vec<Tested>> {
    let mut doc = doc.clone();
    let mut wanted = Vec::new();
    for resource in doc["resources"].as_array_mut().into_iter().flatten() {
        if let Some(entries) = crate::required::of(resource)? {
            if let Some(properties) = resource["properties"].as_object_mut() {
                properties.remove(crate::required::PROPERTY);
                properties.insert("reportVersions".into(), "latest".into());
            }
            wanted.push((resource.clone(), entries));
        }
    }
    let result = configurator(&doc)?.invoke_test()?;
    let mut states = Vec::new();
    flatten(&result.results, &mut states);
    for t in &mut states {
        if let Some((resource, entries)) = wanted.iter().find(|(r, _)| r["name"] == t.name.as_str()) {
            t.judged = Some(judge(&doc, resource, entries, &t.actual)?);
        }
    }
    Ok(states)
}

/// A resource's required versions, judged by what it reports.
struct Judged {
    decision: Decision,
    /// What the app shows (see [`crate::required::assess`]).
    shown: Option<crate::required::Required>,
    /// The installed version as reported; `Some("?")`: installed, version unknown.
    installed: Option<String>,
}

/// Judge a resource's versions from its state (with `reportVersions: latest`). Asks it for every
/// version it can install if the decision depends on them.
fn judge(doc: &Value, resource: &Value, entries: &[crate::required::Entry], state: &Value) -> Result<Judged> {
    let kind = resource["type"].as_str().unwrap_or_default();
    let mut facts = crate::required::Facts::from_state(kind, state);
    if facts.need_list(entries) {
        let all = get_state(doc, resource, "all")?;
        facts.available = Some(crate::required::Facts::from_state(kind, &all).available.unwrap_or_default());
    }
    let (decision, shown) = crate::required::assess(resource["name"].as_str().unwrap_or_default(), entries, &facts);
    Ok(Judged { decision, shown, installed: facts.installed })
}

/// One resource's state from DSC's get, with `reportVersions` (`latest` or `all`).
fn get_state(doc: &Value, resource: &Value, report: &str) -> Result<Value> {
    let mut resource = resource.clone();
    if let Some(r) = resource.as_object_mut() {
        r.remove("dependsOn");
    }
    if let Some(properties) = resource["properties"].as_object_mut() {
        properties.remove(crate::required::PROPERTY);
        properties.insert("reportVersions".into(), report.into());
    }
    let mut single = doc.clone();
    single["resources"] = Value::Array(vec![resource]);
    let result = configurator(&single)?.invoke_get()?;
    match result.results.first().map(|r| &r.result) {
        Some(GetResult::Resource(r)) => Ok(r.actual_state.clone()),
        _ => anyhow::bail!("no state from the resource"),
    }
}

/// A single-resource step document ready to run: without `versions`, and with `targetVersion`
/// where the required versions call for one. The second value is set when no acceptable version
/// is offered: the step must not run. The caller holds the engine lock.
fn resolve(step_doc: &Value) -> Result<(Value, Option<crate::required::Required>)> {
    let mut doc = step_doc.clone();
    let resource = &doc["resources"][0];
    let entries = match crate::required::of(resource)? {
        Some(entries) if applies_here(resource, step_doc) && exists(&resource["properties"]) => entries,
        _ => {
            crate::required::strip(&mut doc["resources"]);
            return Ok((doc, None));
        }
    };
    crate::required::strip(&mut doc["resources"]);
    let state = get_state(&doc, &doc["resources"][0], "latest")?;
    let judged = judge(&doc, &doc["resources"][0].clone(), &entries, &state)?;
    match judged.decision {
        Decision::Exactly(version) => doc["resources"][0]["properties"]["targetVersion"] = version.into(),
        Decision::NoneOffered => return Ok((doc, judged.shown)),
        Decision::Free => {}
    }
    Ok((doc, None))
}

/// The preview of a step, with its required version resolved where furnish can tell.
fn preview_resolved(step_doc: &Value) -> Result<Step> {
    match resolve(step_doc) {
        Ok((doc, None)) => preview(&doc),
        Ok((doc, Some(required))) => {
            let mut step = preview(&doc)?;
            (step.action, step.changes) = (Action::Nothing, vec![]);
            step.details = vec![format!("No acceptable version: needs {}", required.summary)];
            Ok(step)
        }
        // Found out when the step runs (a tool an earlier step installs, say).
        Err(_) => {
            let mut doc = step_doc.clone();
            crate::required::strip(&mut doc["resources"]);
            preview(&doc)
        }
    }
}

/// The app's own removal steps (`metadata.furnish.uninstall`): DSC resources in the format of
/// `resources`, which uninstalling runs instead of setting the app's resources to `_exist: false`.
/// DSC ignores what is in `metadata`, so the document stays a valid DSC document.
pub fn removal_steps(doc: &Value) -> Option<&Vec<Value>> {
    doc["metadata"]["furnish"]["uninstall"].as_array()
}

/// The document that uninstalls with the app's removal steps: the app's document, with them as
/// its resources. `None` if the app has none.
pub fn removal_doc(doc: &Value) -> Option<Value> {
    let steps = removal_steps(doc)?;
    let mut removal = doc.clone();
    removal["resources"] = Value::Array(steps.clone());
    Some(removal)
}

/// Why furnish can't uninstall the app here, e.g. "the resource 'profile'
/// (Microsoft.DSC.Transitional/RunCommandOnSet)"; `None` if it can. It can't if the app has no
/// removal steps and a resource that applies on this computer can't remove what it set up (see
/// [`can_remove`]). A type DSC doesn't find here doesn't count: uninstalling then fails before any
/// step runs (every step is previewed first), so nothing is changed and the error names the type.
pub fn unremovable(doc: &Value) -> Result<Option<String>> {
    let _guard = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    unremovable_unlocked(doc)
}

fn unremovable_unlocked(doc: &Value) -> Result<Option<String>> {
    if removal_steps(doc).is_some() {
        return Ok(None);
    }
    let Some(doc) = applicable(doc) else { return Ok(None) };
    let resources = doc["resources"].as_array().cloned().unwrap_or_default();
    let found = capabilities(resources.iter().filter_map(|r| r["type"].as_str()))?;
    Ok(first_unremovable(&resources, |kind| found.iter().find(|(k, _)| k == kind).map(|(_, c)| c.as_slice())))
}

/// The first resource whose type `capabilities` knows and that can't remove. A `Furnish/Command`
/// removes only with its `remove` command.
fn first_unremovable<'a>(resources: &[Value], capabilities: impl Fn(&str) -> Option<&'a [Capability]>) -> Option<String> {
    resources.iter().find_map(|r| {
        let kind = r["type"].as_str().unwrap_or_default();
        let name = r["name"].as_str().unwrap_or("?");
        let command_without_remove = kind.eq_ignore_ascii_case("Furnish/Command") && r["properties"]["remove"].is_null();
        (command_without_remove || !can_remove(capabilities(kind)?)).then(|| format!("the resource '{name}' ({kind})"))
    })
}

/// Whether a resource with these capabilities removes what it set up when furnish sets it to
/// `_exist: false`. DSC then calls its `set` if the manifest says `set.handlesExist: true`, else
/// its `delete` if it has one, else fails ("does not support `delete` and does not handle `_exist`
/// as false"). Resources that have neither may still list `_exist` in their schema (File/Content).
fn can_remove(capabilities: &[Capability]) -> bool {
    capabilities.contains(&Capability::SetHandlesExist) || capabilities.contains(&Capability::Delete)
}

/// Whether DSC's resource of this type on this computer can remove (see [`can_remove`]); `None`
/// if DSC doesn't find it here.
pub fn removes(kind: &str) -> Result<Option<bool>> {
    let _guard = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    Ok(capabilities([kind])?.first().map(|(_, c)| can_remove(c)))
}

/// The capabilities of the resource types DSC finds here, from their manifests: (type as given, capabilities).
fn capabilities<'a>(kinds: impl IntoIterator<Item = &'a str>) -> Result<Vec<(String, Vec<Capability>)>> {
    ensure_own_resources()?;
    let parsed: Vec<(&str, FullyQualifiedTypeName)> = kinds.into_iter().filter_map(|k| Some((k, k.parse().ok()?))).collect();
    let filters: Vec<DiscoveryFilter> = parsed.iter().map(|(_, t)| DiscoveryFilter::new(t, None, None)).collect();
    let mut discovery = Discovery::new();
    discovery.find_resources(&filters, ProgressFormat::None)?;
    let mut found = Vec::new();
    for ((kind, _), filter) in parsed.iter().zip(&filters) {
        if let Some(resource) = discovery.find_resource(filter)? {
            found.push((kind.to_string(), resource.capabilities.clone()));
        }
    }
    Ok(found)
}

/// A resource type DSC doesn't find on this computer, e.g. "the DSC resource Microsoft.Windows/Registry".
fn missing_resource(error: &anyhow::Error) -> Option<String> {
    match error.downcast_ref::<DscError>()? {
        DscError::ResourceNotFound(kind, _) => Some(format!("the DSC resource {kind}")),
        _ => None,
    }
}

/// The document without the resources whose `condition` is false here, and without
/// `dependsOn` entries naming those; `None` if no resource is left. furnish skips them itself
/// because DSC wants every resource type of a document installed, also those it would skip,
/// e.g. a Windows-only resource on Linux.
fn applicable(doc: &Value) -> Option<Value> {
    let resources = doc["resources"].as_array().cloned().unwrap_or_default();
    let (kept, dropped): (Vec<Value>, Vec<Value>) = resources.into_iter().partition(|r| applies_here(r, doc));
    if kept.is_empty() {
        return None;
    }
    let dropped: Vec<&str> = dropped.iter().filter_map(|r| r["name"].as_str()).collect();
    let mut doc = doc.clone();
    doc["resources"] = kept
        .into_iter()
        .map(|mut r| {
            if let Some(deps) = r["dependsOn"].as_array_mut() {
                deps.retain(|d| dependency(d).is_none_or(|n| !dropped.contains(&n.as_str())));
            }
            r
        })
        .collect();
    Some(doc)
}

/// Whether a resource's `condition` holds on this computer with the document's parameter values.
/// A condition furnish can't evaluate is left to DSC. A Furnish/NixOS step with `onlyIfSetUp`
/// doesn't apply where its place isn't set up (`nixos_apps::not_needed_here`).
pub fn applies_here(resource: &Value, doc: &Value) -> bool {
    if crate::nixos_apps::not_needed_here(resource).is_some() {
        return false;
    }
    let Some(condition) = resource["condition"].as_str() else { return true };
    let values: serde_json::Map<String, Value> = doc["parameters"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(name, p)| {
            let value = match &p["defaultValue"] {
                // `options::apply` escapes a leading `[` as `[[`, as DSC expects.
                Value::String(s) if s.starts_with("[[") => Value::String(s[1..].to_string()),
                Value::Null => return None,
                v => v.clone(),
            };
            Some((name.clone(), value))
        })
        .collect();
    crate::options::condition_holds(condition, &values).unwrap_or(true)
}

/// Install an app, or bring it up to date.
pub fn apply(doc: &Value) -> Result<()> {
    run(doc, false, |_| {})
}

/// Uninstall an app: its removal steps if it has them (see [`removal_steps`]), else every
/// resource set to `_exist: false`, in reverse install order.
pub fn remove(doc: &Value) -> Result<()> {
    run(doc, true, |_| {})
}

/// What one resource would do (DSC what-if), or did.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub action: Action,
    /// The resource's own description of its steps (DSC's `_metadata.whatIf`), e.g. the exact command.
    pub details: Vec<String>,
    /// The same as notes the GUI translates, from furnish's configuration resources
    /// (`_metadata.furnish`): `{code, params}`, see resources/note.rs.
    #[serde(default)]
    pub notes: Vec<Value>,
    /// Properties that would change, other than presence (`_exist`).
    pub changes: Vec<Change>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    Install,
    Remove,
    Change,
    Nothing,
    /// Its `condition` excludes it on this machine, or it isn't needed here (`onlyIfSetUp`).
    Skipped,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Change {
    pub property: String,
    pub before: Value,
    pub after: Value,
}

/// Progress of an install/update/uninstall, reported while it runs.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "event", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Progress {
    /// The steps that will run, in order (same as `plan`).
    Planned { steps: Vec<Step> },
    Started { index: usize, at_ms: u64 },
    Finished { index: usize, at_ms: u64, duration_ms: u64, outcome: Outcome },
}

/// What a step actually did.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Outcome {
    pub action: Action,
    pub changes: Vec<Change>,
    /// Set if the step failed; the run stops there.
    pub error: Option<String>,
    /// The user cancelled this step (refused the password dialog or force-stopped it).
    pub cancelled: bool,
}

/// Preview installing (or uninstalling) an app without changing anything.
pub fn plan(doc: &Value, uninstall: bool) -> Result<Vec<Step>> {
    let _guard = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    let _working = crate::load::activity(); // load-aware updates ignore what furnish causes
    step_docs(doc, uninstall)?.iter().map(preview_resolved).collect()
}

/// Install (or uninstall) one resource at a time, reporting each step as it starts and ends.
/// furnish orchestrates the steps itself because DSC only reports per-resource progress on stderr.
/// Fails with [`Cancelled`] if the user aborted, force-stopped or refused the password dialog.
pub fn run(doc: &Value, uninstall: bool, mut report: impl FnMut(Progress)) -> Result<()> {
    let _guard = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    let _working = crate::load::activity(); // load-aware updates ignore what furnish causes
    ABORT.store(false, Ordering::SeqCst);
    if let Some(file) = STOP_FILE.get() {
        let _ = fs::remove_file(file);
    }
    let docs = step_docs(doc, uninstall)?;
    let steps = docs.iter().map(preview_resolved).collect::<Result<Vec<_>>>()?;
    report(Progress::Planned { steps: steps.clone() });
    for (index, (step_doc, step)) in docs.iter().zip(&steps).enumerate() {
        if ABORT.load(Ordering::SeqCst) {
            return Err(Cancelled.into());
        }
        let started = now_ms();
        if step.action == Action::Skipped {
            let outcome = Outcome { action: Action::Skipped, changes: vec![], error: None, cancelled: false };
            report(Progress::Finished { index, at_ms: started, duration_ms: 0, outcome });
            continue;
        }
        report(Progress::Started { index, at_ms: started });
        // The required version, decided right before the step: earlier steps may have changed it.
        let result = resolve(step_doc).and_then(|(step_doc, none_offered)| match none_offered {
            Some(required) => Err(anyhow::anyhow!("'{}' ({}): needs {}", step.name, step.kind, required.summary)),
            None => configurator(&step_doc).and_then(|mut c| Ok(c.invoke_set(false)?)),
        });
        let finished = now_ms();
        let outcome = match &result {
            Ok(r) => match r.results.first().map(|r| &r.result) {
                Some(SetResult::Resource(r)) => Outcome {
                    // A step without preview ran its command, even if its state reads the same.
                    action: match action(&r.before_state, &r.after_state) {
                        Action::Nothing if step.details.first().is_some_and(|d| d == NO_PREVIEW) => Action::Change,
                        // A configuration resource changed what its properties don't show (see preview).
                        Action::Nothing if !step.notes.is_empty() && step.action != Action::Nothing => step.action,
                        a => a,
                    },
                    changes: changes(&r.before_state, &r.after_state),
                    error: failed_command(&r.before_state, &r.after_state)
                        .map(|failure| format!("'{}' ({}): {failure}", step.name, step.kind)),
                    cancelled: false,
                },
                _ => Outcome { action: Action::Skipped, changes: vec![], error: None, cancelled: false },
            },
            Err(e) => {
                let error = format!("{e:#}");
                let cancelled = is_cancellation(&error);
                Outcome { action: step.action, changes: vec![], error: (!cancelled).then_some(error), cancelled }
            }
        };
        let cancelled = outcome.cancelled;
        let failure = result.is_ok().then(|| outcome.error.clone()).flatten();
        report(Progress::Finished { index, at_ms: finished, duration_ms: finished - started, outcome });
        if cancelled {
            return Err(Cancelled.into());
        }
        result?;
        if let Some(failure) = failure {
            emit_log("error", failure.clone());
            anyhow::bail!(failure);
        }
    }
    Ok(())
}

/// Why a step whose set DSC reports as successful failed: the command it ran exited with a code
/// other than the expected one, e.g. "`rm -f /x` exited with code 1".
///
/// DSC 3.3's Microsoft.DSC.Transitional/RunCommandOnSet exits 0 whatever its command returns. Its
/// result (the after-state) has the command's exit code as `exitCode`, left out if 0. Its input's
/// `exitCode` is the code the author expects (default 0), which its get echoes into the
/// before-state, but it never compares the two. So a step fails if its after-state has an
/// integer `exitCode` other than its before-state's (default 0). This holds for any resource
/// that reports `exitCode` this way; among DSC's own resources only RunCommandOnSet has it, and
/// a resource whose `exitCode` describes something that a set doesn't change keeps the same
/// value before and after. RunCommandOnSet drops the command's output (it traces it only at a
/// level DSC can't turn on), so the error can't show it.
fn failed_command(before: &Value, after: &Value) -> Option<String> {
    let code = after.get("exitCode")?.as_i64()?;
    let expected = before.get("exitCode").and_then(Value::as_i64).unwrap_or(0);
    if code == expected {
        return None;
    }
    let mut command: Vec<String> = after["executable"].as_str().map(String::from).into_iter().collect();
    command.extend(after["arguments"].as_array().into_iter().flatten().map(|a| match a {
        Value::String(s) => s.clone(),
        v => v.to_string(),
    }));
    let what = if command.is_empty() { "its command".to_string() } else { format!("`{}`", command.join(" ")) };
    let expected = if expected == 0 { String::new() } else { format!(", expected {expected}") };
    Some(format!("{what} exited with code {code}{expected}"))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// One single-resource document per step, in run order. Uninstall runs the app's removal steps
/// as they are, in their order, if it has them. Else it flips `_exist` and runs the install order
/// backwards, so dependents go before what they depend on.
fn step_docs(doc: &Value, uninstall: bool) -> Result<Vec<Value>> {
    let (doc, flip) = match uninstall.then(|| removal_doc(doc)).flatten() {
        Some(removal) => (removal, false),
        None => (doc.clone(), uninstall),
    };
    let resources = doc["resources"].as_array().cloned().unwrap_or_default();
    let mut order = invocation_order(&resources)?;
    if flip {
        order.reverse();
    }
    Ok(order
        .into_iter()
        .map(|i| {
            let mut resource = resources[i].clone();
            if let Some(r) = resource.as_object_mut() {
                // Ordering is handled here; DSC would reject references to absent resources.
                r.remove("dependsOn");
            }
            if flip {
                resource["properties"]["_exist"] = Value::Bool(false);
            }
            let mut single = doc.clone();
            single["resources"] = Value::Array(vec![resource]);
            single
        })
        .collect())
}

/// DSC's order: document order, except that a dependency runs right before the first
/// resource that needs it.
fn invocation_order(resources: &[Value]) -> Result<Vec<usize>> {
    fn visit(i: usize, resources: &[Value], order: &mut Vec<usize>, path: &mut Vec<usize>) -> Result<()> {
        if order.contains(&i) {
            return Ok(());
        }
        if path.contains(&i) {
            anyhow::bail!("dependsOn cycle involving '{}'", resources[i]["name"].as_str().unwrap_or_default());
        }
        path.push(i);
        for dep in dependencies(&resources[i]) {
            let Some(d) = resources.iter().position(|r| r["name"] == dep.as_str()) else {
                anyhow::bail!("'{}' depends on unknown resource '{dep}'", resources[i]["name"].as_str().unwrap_or_default());
            };
            visit(d, resources, order, path)?;
        }
        path.pop();
        order.push(i);
        Ok(())
    }
    let mut order = Vec::new();
    for i in 0..resources.len() {
        visit(i, resources, &mut order, &mut Vec::new())?;
    }
    Ok(order)
}

/// Names from `dependsOn: ["[resourceId('Type', 'name')]"]`.
fn dependencies(resource: &Value) -> Vec<String> {
    resource["dependsOn"].as_array().into_iter().flatten().filter_map(dependency).collect()
}

/// The name in `[resourceId('Type', 'name')]`.
fn dependency(entry: &Value) -> Option<String> {
    let args = entry.as_str()?.strip_prefix("[resourceId(")?.strip_suffix(")]")?;
    let (_, name) = args.split_once(',')?;
    Some(name.trim().trim_matches('\'').to_string())
}

/// The first detail of a step DSC can't preview (see `preview`).
const NO_PREVIEW: &str = "No preview: this step runs its command or script";

/// What-if for one single-resource document.
fn preview(doc: &Value) -> Result<Step> {
    let resource = &doc["resources"][0];
    let name = resource["name"].as_str().unwrap_or_default().to_string();
    let kind = resource["type"].as_str().unwrap_or_default().to_string();
    let skipped = || {
        let details = match crate::nixos_apps::not_needed_here(resource) {
            Some(why) => vec![format!("Not needed here: {why}")],
            None => resource["condition"].as_str().map(|c| format!("Not on this system: {c}")).into_iter().collect(),
        };
        Step { name: name.clone(), kind: kind.clone(), action: Action::Skipped, details, notes: vec![], changes: vec![] }
    };
    if !applies_here(resource, doc) {
        return Ok(skipped());
    }
    let mut configurator = configurator(doc)?;
    configurator.context.execution_type = ExecutionKind::WhatIf;
    let result = match configurator.invoke_set(false) {
        Ok(result) => result,
        // Resources that decide themselves whether to change something (`set.implementsPretest`)
        // and have no what-if, e.g. RunCommandOnSet and the PowerShell script resources: DSC
        // can't preview them. They run their command or script when the step runs.
        Err(DscError::NotImplemented(_)) => {
            let mut details = vec![NO_PREVIEW.to_string()];
            details.extend(property_lines(&resource["properties"]));
            return Ok(Step { name, kind, action: Action::Change, details, notes: vec![], changes: vec![] });
        }
        Err(e) => return Err(e.into()),
    };
    // A resource skipped by its condition produces no result at all.
    let Some(r) = result.results.first() else {
        return Ok(skipped());
    };
    let mut details: Vec<String> = r.metadata.as_ref()
        .and_then(|m| m.other.get("whatIf"))
        .and_then(Value::as_array)
        .map(|lines| lines.iter().filter_map(|l| l.as_str().map(String::from)).collect())
        .unwrap_or_default();
    // furnish's resources describe their exact commands; for others, show what they are given.
    if !crate::catalog::RESOURCE_TYPES.contains(&kind.as_str()) {
        details.extend(property_lines(&resource["properties"]));
    }
    let (before, after) = match &r.result {
        SetResult::Resource(r) => (r.before_state.clone(), r.after_state.clone()),
        SetResult::Group(_) => (Value::Null, Value::Null),
    };
    let other = r.metadata.as_ref().map(|m| &m.other);
    let notes = other.and_then(|m| m.get("furnish")).and_then(Value::as_array).cloned().unwrap_or_default();
    // furnish's configuration resources don't show whole files as properties, so a change can
    // leave the properties shown as they were; they say so (see resources::run).
    let action = match action(&before, &after) {
        Action::Nothing if other.and_then(|m| m.get("furnishChange")) == Some(&Value::Bool(true)) => Action::Change,
        a => a,
    };
    Ok(Step { name, kind, action, details, notes, changes: changes(&before, &after) })
}

/// One line per property, `name: value`, shortened to a line's length.
fn property_lines(properties: &Value) -> Vec<String> {
    const MAX: usize = 200;
    properties.as_object().into_iter().flatten()
        .filter(|(k, _)| !k.starts_with('_'))
        .map(|(k, v)| {
            let value = match v { Value::String(s) => s.clone(), v => v.to_string() };
            let line = format!("{k}: {}", value.replace('\n', " ⏎ "));
            match line.char_indices().nth(MAX) {
                Some((cut, _)) => format!("{}…", &line[..cut]),
                None => line,
            }
        })
        .collect()
}

fn exists(state: &Value) -> bool {
    !(state["_exist"] == false || state["_exist"] == "false")
}

fn changes(before: &Value, after: &Value) -> Vec<Change> {
    let Some(after_map) = after.as_object() else { return vec![] };
    after_map.iter()
        .filter(|(k, v)| !k.starts_with('_') && before.get(k.as_str()) != Some(*v))
        .map(|(k, v)| Change { property: k.clone(), before: before.get(k.as_str()).cloned().unwrap_or(Value::Null), after: v.clone() })
        .collect()
}

fn action(before: &Value, after: &Value) -> Action {
    match (exists(before), exists(after)) {
        (false, true) => Action::Install,
        (true, false) => Action::Remove,
        (false, false) => Action::Nothing,
        (true, true) if changes(before, after).is_empty() => Action::Nothing,
        (true, true) => Action::Change,
    }
}

/// Whether DSC accepts the document (parses it, runs no resource). A resource type that isn't
/// on this computer is fine: it may be where the app is installed (see `available`).
pub fn validate(doc: &Value) -> Result<()> {
    let _guard = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    match configurator(doc) {
        Err(e) if missing_resource(&e).is_some() => Ok(()),
        result => result.map(drop),
    }
}

fn configurator(doc: &Value) -> Result<Configurator> {
    ensure_own_resources()?;
    // A setting changed in the app applies to the next step.
    crate::resource_env::refresh();
    let mut doc = doc.clone();
    // furnish's own property for required versions (see `crate::required`); DSC doesn't know it.
    crate::required::strip(&mut doc["resources"]);
    // DSC would discover resources again while running, after `ensure_own_resources` checked
    // them; furnish skips resources by their condition itself (see `applicable`).
    if let Some(directives) = doc["directives"].as_object_mut() {
        directives.remove("resourceDiscovery");
    }
    let mut configurator = Configurator::new(&doc.to_string(), ProgressFormat::None)?;
    configurator.set_context(None)?;
    Ok(configurator)
}

/// One resource's test result.
struct Tested {
    name: String,
    kind: String,
    /// In the desired state.
    ok: bool,
    actual: Value,
    desired: Value,
    /// Its required versions, if it has any.
    judged: Option<Judged>,
}

impl Tested {
    /// After an install: in the desired state, at the version the required versions call for.
    fn installed_as_required(&self) -> bool {
        match &self.judged {
            None => self.ok,
            Some(Judged { decision: Decision::Exactly(v), installed, .. }) => installed.as_deref() == Some(v.as_str()),
            Some(j) => self.ok && j.shown.as_ref().is_none_or(|r| r.acceptable),
        }
    }
}

/// Collect the test result of every resource, descending into groups.
fn flatten(results: &[ResourceTestResult], out: &mut Vec<Tested>) {
    for result in results {
        match &result.result {
            TestResult::Resource(r) => out.push(Tested {
                name: result.name.clone(),
                kind: result.resource_type.to_string(),
                ok: r.in_desired_state,
                actual: r.actual_state.clone(),
                desired: r.desired_state.clone(),
                judged: None,
            }),
            TestResult::Group(group) => flatten(group, out),
        }
    }
}

/// Everything in desired state → installed; anything missing → not installed; else drifted.
/// A resource that doesn't report `_exist` (not all DSC resources do) counts as missing when it
/// isn't in the desired state. No states at all means every resource was skipped by its `condition`.
fn classify(states: &[(bool, Value)]) -> Status {
    if states.is_empty() {
        return Status::Unavailable;
    }
    let drifted: Vec<_> = states.iter().filter(|(ok, _)| !ok).map(|(_, actual)| actual).collect();
    if drifted.is_empty() {
        Status::Installed
    } else if drifted.iter().any(|a| matches!(&a["_exist"], Value::Bool(false) | Value::Null) || a["_exist"] == "false") {
        Status::NotInstalled
    } else {
        Status::NeedsUpdate
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classify_states() {
        let present = json!({"_exist": true});
        let missing = json!({"_exist": false});
        assert_eq!(classify(&[(true, present.clone())]), Status::Installed);
        assert_eq!(classify(&[(true, present.clone()), (false, missing)]), Status::NotInstalled);
        assert_eq!(classify(&[(false, json!({"_exist": "false"}))]), Status::NotInstalled);
        assert_eq!(classify(&[(false, present)]), Status::NeedsUpdate);
        // Resources without `_exist`, e.g. a script resource whose test failed: not installed yet.
        assert_eq!(classify(&[(false, json!({"name": "Acme VPN"}))]), Status::NotInstalled);
        assert_eq!(classify(&[]), Status::Unavailable);
    }

    fn tested(desired: Value, actual: Value) -> Tested {
        Tested { name: "pkg".into(), kind: "Furnish/Package".into(), ok: true, actual, desired, judged: None }
    }

    #[test]
    fn packages_installed_outside_furnish_say_where_from() {
        let nixos = external(&[tested(
            json!({"name": "node", "nixName": "nodejs_22"}),
            json!({"name": "node", "_exist": true, "managedBy": "nixos", "declaredIn": "/etc/nixos"}),
        )])
        .unwrap();
        assert_eq!(nixos.summary(), "in your NixOS configuration (/etc/nixos)");
        assert_eq!(nixos.reason(), "it is in your NixOS configuration (/etc/nixos)");
        assert_eq!(nixos.refusal("node"), "node is in your NixOS configuration (/etc/nixos): remove nodejs_22 there");

        let other = external(&[tested(
            json!({"name": "claude-code", "nixName": "claude-code-bin", "command": "claude"}),
            json!({"name": "claude-code", "_exist": true, "managedBy": "other", "path": "/opt/homebrew/bin/claude"}),
        )])
        .unwrap();
        assert_eq!(other.package, "claude-code", "not the Nix name: Nix didn't install it");
        assert_eq!(other.summary(), "installed outside furnish: /opt/homebrew/bin/claude");
        assert_eq!(other.reason(), "it was installed outside furnish (/opt/homebrew/bin/claude)");
        assert_eq!(
            other.refusal("claude-code"),
            "claude-code was installed outside furnish (/opt/homebrew/bin/claude): uninstall it the way it was installed"
        );

        // A package in a nix profile is furnish's (or the user's) own, not external.
        assert_eq!(external(&[tested(json!({"name": "jq"}), json!({"name": "jq", "_exist": true, "managedBy": "profile"}))]), None);
    }

    #[test]
    fn a_command_that_exits_with_another_code_fails_its_step() {
        // RunCommandOnSet's before-state echoes the input; its after-state has the exit code.
        let before = json!({"executable": "sh", "arguments": ["-c", "exit 3"]});
        let after = json!({"executable": "sh", "arguments": ["-c", "exit 3"], "exitCode": 3});
        assert_eq!(failed_command(&before, &after).as_deref(), Some("`sh -c exit 3` exited with code 3"));
        assert_eq!(failed_command(&before, &json!({"executable": "true"})), None, "0 is left out");
        // The author expects 3: 3 is fine, 0 isn't.
        let expects = json!({"executable": "x", "exitCode": 3});
        assert_eq!(failed_command(&expects, &json!({"executable": "x", "exitCode": 3})), None);
        assert_eq!(failed_command(&expects, &json!({"executable": "x"})), None, "no exitCode in the result, nothing to compare");
        assert_eq!(failed_command(&expects, &json!({"executable": "x", "exitCode": 0})).as_deref(), Some("`x` exited with code 0, expected 3"));
        // Other resources: no exitCode, or one the set doesn't change.
        assert_eq!(failed_command(&json!({"_exist": false}), &json!({"_exist": true})), None);
        assert_eq!(failed_command(&json!({"exitCode": 1077}), &json!({"exitCode": 1077})), None);
        assert_eq!(failed_command(&json!({}), &json!({"exitCode": 2})).as_deref(), Some("its command exited with code 2"));
    }

    #[test]
    fn missing_tool_reason_from_resource_error() {
        let err = "Command: Resource 'furnish-package.dsc.resource.sh' [exit code 11] furnish: needs Homebrew (brew)\nmore";
        assert_eq!(missing_reason(err).as_deref(), Some("Homebrew (brew)"));
        assert_eq!(missing_reason("exit code 1: E: Unable to locate package"), None);
    }

    #[test]
    fn run_order_puts_dependencies_first_and_reverses_for_uninstall() {
        let doc = json!({"resources": [
            {"name": "app", "type": "T/X", "dependsOn": ["[resourceId('T/X', 'runtime')]"], "properties": {}},
            {"name": "runtime", "type": "T/X", "properties": {}},
            {"name": "shortcut", "type": "T/X", "properties": {}},
        ]});
        let names = |docs: Vec<Value>| docs.iter().map(|d| d["resources"][0]["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(names(step_docs(&doc, false).unwrap()), ["runtime", "app", "shortcut"]);
        let uninstall = step_docs(&doc, true).unwrap();
        assert_eq!(names(uninstall.clone()), ["shortcut", "app", "runtime"]);
        assert_eq!(uninstall[0]["resources"][0]["properties"]["_exist"], false);
        assert!(uninstall[1]["resources"][0].get("dependsOn").is_none());
        let cycle = json!({"resources": [
            {"name": "a", "dependsOn": ["[resourceId('T/X', 'b')]"]},
            {"name": "b", "dependsOn": ["[resourceId('T/X', 'a')]"]},
        ]});
        assert!(step_docs(&cycle, false).is_err());
    }

    #[test]
    fn removal_steps_run_as_they_are_instead_of_the_reversed_resources() {
        let other = if cfg!(windows) { "Linux" } else { "Windows" };
        let doc = json!({
            "parameters": {"server": {"type": "string", "defaultValue": "vpn.acme.example"}},
            "metadata": {"furnish": {"uninstall": [
                {"name": "reload", "type": "T/Cmd", "dependsOn": ["[resourceId('T/Cmd', 'delete')]"], "properties": {"executable": "nmcli"}},
                {"name": "delete", "type": "T/Cmd", "properties": {"executable": "rm"}},
                {"name": "elsewhere", "type": "T/Cmd", "condition": format!("[equals(context().os.family, '{other}')]"), "properties": {}},
            ]}},
            "resources": [{"name": "profile", "type": "T/File", "properties": {"path": "/etc/x"}}],
        });
        let docs = step_docs(&doc, true).unwrap();
        let names: Vec<_> = docs.iter().map(|d| d["resources"][0]["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["delete", "reload", "elsewhere"]);
        assert!(docs.iter().all(|d| d["resources"][0]["properties"].get("_exist").is_none()), "removal steps run as a normal set");
        assert!(docs[1]["resources"][0].get("dependsOn").is_none());
        // The app's parameters stay, so the steps' expressions and conditions see the options.
        assert_eq!(docs[0]["parameters"], doc["parameters"]);
        assert!(!applies_here(&docs[2]["resources"][0], &docs[2]), "conditions apply to removal steps");
        // Installing still runs the resources.
        let install = step_docs(&doc, false).unwrap();
        assert_eq!((install.len(), &install[0]["resources"][0]["name"]), (1, &json!("profile")));
        assert_eq!(removal_doc(&doc).unwrap()["resources"].as_array().unwrap().len(), 3);
        assert!(removal_doc(&json!({"resources": []})).is_none());
    }

    #[test]
    fn an_app_is_removable_if_its_resources_handle_exist_or_delete() {
        use Capability::*;
        let caps = |kind: &str| -> Option<&'static [Capability]> {
            match kind {
                "Furnish/Package" => Some(&[Get, Set, SetHandlesExist, Test]),
                "Microsoft.Windows/Registry" => Some(&[Get, Set, Delete]),
                "Microsoft.Filesystem.File/Content" => Some(&[Get, Set, Test]),
                "Microsoft.DSC.Transitional/RunCommandOnSet" => Some(&[Get, Set]),
                _ => None, // not on this computer
            }
        };
        let resources = |list: &[(&str, &str)]| -> Vec<Value> {
            list.iter().map(|(name, kind)| json!({"name": name, "type": kind})).collect()
        };
        assert!(can_remove(&[Get, Set, SetHandlesExist]));
        assert!(can_remove(&[Get, Delete]));
        assert!(!can_remove(&[Get, Set, Test]));
        assert_eq!(first_unremovable(&resources(&[("app", "Furnish/Package"), ("key", "Microsoft.Windows/Registry")]), caps), None);
        assert_eq!(
            first_unremovable(&resources(&[("app", "Furnish/Package"), ("profile", "Microsoft.Filesystem.File/Content")]), caps).as_deref(),
            Some("the resource 'profile' (Microsoft.Filesystem.File/Content)")
        );
        // A type DSC doesn't find here: uninstalling fails before any step, so it doesn't count.
        assert_eq!(first_unremovable(&resources(&[("vpn", "Acme/Vpn")]), caps), None);
        // With removal steps, the resources don't matter (and nothing is looked up).
        let doc = json!({"metadata": {"furnish": {"uninstall": [{"name": "rm", "type": "Microsoft.DSC.Transitional/RunCommandOnSet"}]}},
                         "resources": [{"name": "profile", "type": "Microsoft.Filesystem.File/Content"}]});
        assert_eq!(unremovable_unlocked(&doc).unwrap(), None);
        // Resources that don't apply on this computer don't count either.
        let other = if cfg!(windows) { "Linux" } else { "Windows" };
        let doc = json!({"resources": [{"name": "s", "type": "Microsoft.DSC.Transitional/WindowsPowerShellScript",
                                        "condition": format!("[equals(context().os.family, '{other}')]")}]});
        assert_eq!(unremovable_unlocked(&doc).unwrap(), None);
    }

    #[test]
    fn plan_actions_from_before_and_after_state() {
        let missing = json!({"name": "x", "_exist": false});
        let present = json!({"name": "x", "_exist": true, "version": "1"});
        let newer = json!({"name": "x", "_exist": true, "version": "2"});
        assert_eq!(action(&missing, &json!({"name": "x", "_exist": true})), Action::Install);
        assert_eq!(action(&present, &missing), Action::Remove);
        assert_eq!(action(&present, &present), Action::Nothing);
        assert_eq!(action(&present, &newer), Action::Change);
        assert_eq!(changes(&present, &newer)[0].property, "version");
    }

    #[test]
    fn log_lines_keep_resource_output_and_dsc_progress_only() {
        use tracing::Level;
        let line = |level, msg| log_line(level, msg).map(|l| (l.level, l.message));
        assert_eq!(line(Level::TRACE, "PID 42: resolving dependencies..."), Some(("output", "resolving dependencies...".into())));
        assert_eq!(line(Level::INFO, "PID 42: Running as administrator: pacman -S x"), Some(("info", "Running as administrator: pacman -S x".into())));
        assert_eq!(line(Level::ERROR, "PID 7: no supported package manager"), Some(("error", "no supported package manager".into())));
        assert_eq!(line(Level::WARN, "Resource is deprecated"), Some(("warn", "Resource is deprecated".into())));
        assert_eq!(line(Level::DEBUG, "Discovering resources in /tmp"), None);
        assert_eq!(line(Level::TRACE, "PID 42:   "), None);
    }

    #[test]
    fn furnish_types_from_elsewhere_are_refused() {
        let bundled = env::temp_dir().join(format!("furnish-bundled-{}", std::process::id()));
        fs::create_dir_all(&bundled).unwrap();
        let own = |file: &str| bundled.join(file);
        let manifests = vec![
            ("Furnish/Package".to_string(), own("furnish-package.dsc.resource.json")),
            ("Furnish/Installer".to_string(), own("furnish-installer.dsc.resource.json")),
            ("Microsoft.Windows/Registry".to_string(), PathBuf::from("/opt/dsc/registry.dsc.resource.json")),
        ];
        assert!(foreign_furnish_manifests(&manifests, &bundled).is_empty());
        // Any version, any case: DSC would prefer a higher version, or the first found if equal.
        let mut shadowed = manifests.clone();
        shadowed.push(("furnish/package".into(), PathBuf::from("/opt/homebrew/bin/evil.dsc.resource.json")));
        let foreign = foreign_furnish_manifests(&shadowed, &bundled);
        assert_eq!(foreign.len(), 1);
        assert_eq!(foreign[0].1, PathBuf::from("/opt/homebrew/bin/evil.dsc.resource.json"));
        fs::remove_dir_all(bundled).unwrap();
    }

    #[test]
    fn dscs_own_resources_come_first_and_cant_be_shadowed() {
        let tmp = env::temp_dir().join(format!("furnish-discovery-{}", std::process::id()));
        let folder = |name: &str, files: &[(&str, &str)]| {
            let d = tmp.join(name);
            fs::create_dir_all(&d).unwrap();
            for (file, contents) in files {
                fs::write(d.join(file), contents).unwrap();
            }
            d
        };
        let own = folder("own", &[]);
        let dsc = folder("dsc", &[
            ("group.dsc.resource.json", r#"{"type": "Microsoft.DSC/Group", "version": "0.1.0"}"#),
            ("registry.dsc.manifests.json", r#"{"resources": [{"type": "Microsoft.Windows/Registry", "version": "1.0.0"}]}"#),
        ]);
        // A higher version of a type furnish ships (any case), in a manifest list or a YAML file.
        let newer = folder("newer", &[("x.dsc.manifests.yaml", "resources:\n  - type: microsoft.windows/registry\n    version: 9.0.0\n")]);
        let fake = folder("fake", &[("group.dsc.resource.json", r#"{"type": "Microsoft.DSC/Group", "version": "99.0.0"}"#)]);
        let other = folder("other", &[("vpn.dsc.resource.json", r#"{"type": "Acme/Vpn", "version": "1.0.0"}"#), ("readme.txt", "not a manifest")]);
        let others = vec![newer.clone(), dsc.clone(), other.clone(), fake.clone()];

        let (programs, resources) = discovery_dirs(&own, Some(&dsc), others.clone());
        assert_eq!(resources, [own.clone(), dsc.clone(), other.clone()]);
        assert_eq!(programs, [own.clone(), dsc.clone(), newer.clone(), other.clone(), fake.clone()]);

        // Without DSC's resources (e.g. a plain `cargo build`), DSC installed elsewhere counts.
        let (programs, resources) = discovery_dirs(&own, None, others);
        assert_eq!(resources, programs);
        assert_eq!(resources[..2], [own, newer]);
        fs::remove_dir_all(tmp).unwrap();
    }

    #[test]
    fn shipped_dsc_resources_match_the_embedded_dsc() {
        let dsc: Value = serde_json::from_str(include_str!("../packaging/dsc.json")).unwrap();
        let version = dsc["version"].as_str().unwrap();
        let tag = format!(r#"dsc-lib = {{ git = "https://github.com/PowerShell/DSC", tag = "v{version}" }}"#);
        assert!(include_str!("../Cargo.toml").contains(&tag), "packaging/dsc.json has DSC {version}, Cargo.toml another dsc-lib");
        let commit = dsc["commit"].as_str().unwrap();
        assert!(include_str!("../Cargo.lock").contains(&format!("?tag=v{version}#{commit}")), "packaging/dsc.json: commit");
    }

    #[test]
    fn the_service_searches_only_folders_admins_control() {
        let path = vec![PathBuf::from("/usr/bin"), PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/bin"), PathBuf::new()];
        let admin_only = |d: &Path| d != Path::new("/opt/homebrew/bin");
        let user = search_dirs(&path, false, admin_only);
        assert_eq!(user[..2], [PathBuf::from("/usr/bin"), PathBuf::from("/opt/homebrew/bin")]);
        assert_eq!(user.iter().filter(|d| *d == Path::new("/usr/bin")).count(), 1, "duplicates");
        let service = search_dirs(&path, true, admin_only);
        assert!(service.contains(&PathBuf::from("/usr/bin")));
        assert!(!service.contains(&PathBuf::from("/opt/homebrew/bin")));
        assert!(service.iter().all(|d| !d.as_os_str().is_empty()));
    }

    #[cfg(unix)]
    #[test]
    fn folders_users_can_write_are_not_admin_only() {
        use std::os::unix::fs::MetadataExt;
        // Build sandboxes (Nix) don't give / to root.
        if fs::metadata("/").unwrap().uid() == 0 {
            assert!(crate::service::admin_only(Path::new("/")));
        }
        // SAFETY: geteuid has no preconditions.
        if unsafe { libc::geteuid() } != 0 {
            let mine = env::temp_dir().join(format!("furnish-admin-only-{}", std::process::id()));
            fs::create_dir_all(&mine).unwrap();
            assert!(!crate::service::admin_only(&mine));
            fs::remove_dir_all(mine).unwrap();
        }
        assert!(!crate::service::admin_only(Path::new("/nonexistent/furnish")));
    }

    #[test]
    fn resources_for_other_systems_are_dropped_before_dsc_sees_them() {
        let other = if cfg!(windows) { "Linux" } else { "Windows" };
        let doc = json!({
            "parameters": { "vpn": { "type": "bool", "defaultValue": true } },
            "resources": [
                {"name": "client", "type": "Furnish/Package", "properties": {"name": "acme-vpn"}},
                {"name": "elsewhere", "type": "Some.Other/Resource", "condition": format!("[equals(context().os.family, '{other}')]")},
                {"name": "profile", "type": "Microsoft.Windows/Registry", "condition": "[parameters('vpn')]",
                 "dependsOn": ["[resourceId('Some.Other/Resource', 'elsewhere')]", "[resourceId('Furnish/Package', 'client')]"]},
            ]
        });
        let kept = applicable(&doc).unwrap();
        let names: Vec<_> = kept["resources"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["client", "profile"]);
        assert_eq!(kept["resources"][1]["dependsOn"], json!(["[resourceId('Furnish/Package', 'client')]"]));
        let none = json!({"resources": [{"name": "x", "type": "T/X", "condition": format!("[equals(context().os.family, '{other}')]")}]});
        assert!(applicable(&none).is_none());
    }

    #[test]
    fn a_nixos_step_only_where_set_up_is_dropped_without_nix_and_its_preview_says_why() {
        if crate::nixos::nix_installed() {
            return; // With Nix, whether it applies depends on how the computer is set up.
        }
        // The owner's example (#8): on a computer without Nix, only the Line step is left.
        let doc = json!({"resources": [
            {"name": "SSH include through Home Manager", "type": "Furnish/NixOS",
             "properties": {"name": "acme-ssh", "onlyIfSetUp": true, "homeOptions": {"programs.ssh.includes": ["config.d/acme.conf"]}}},
            {"name": "SSH include", "type": "Furnish/Line", "dependsOn": ["[resourceId('Furnish/NixOS', 'SSH include through Home Manager')]"],
             "properties": {"path": "~/.ssh/config", "line": "Include config.d/acme.conf", "before": "^\\s*(Host|Match)\\s"}},
        ]});
        let kept = applicable(&doc).unwrap();
        assert_eq!(kept["resources"].as_array().unwrap().len(), 1);
        assert_eq!(kept["resources"][0]["name"], "SSH include");
        assert_eq!(kept["resources"][0]["dependsOn"], json!([]));
        let step = json!({"resources": [doc["resources"][0].clone()]});
        let p = preview(&step).unwrap();
        assert_eq!((p.action, p.details), (Action::Skipped, vec!["Not needed here: Nix isn't installed on this computer".to_string()]));
        // Without the property, the step stays: the resource then says what it needs.
        let mut plain = doc.clone();
        plain["resources"][0]["properties"].as_object_mut().unwrap().remove("onlyIfSetUp");
        assert_eq!(applicable(&plain).unwrap()["resources"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn other_resources_preview_their_properties() {
        let lines = property_lines(&json!({"keyPath": "HKLM\\Software\\Acme", "valueData": {"String": "x"}, "_exist": true, "script": "a\nb"}));
        assert!(lines.contains(&"keyPath: HKLM\\Software\\Acme".to_string()), "{lines:?}");
        assert!(lines.contains(&r#"valueData: {"String":"x"}"#.to_string()), "{lines:?}");
        assert!(lines.contains(&"script: a ⏎ b".to_string()), "{lines:?}");
        assert!(lines.iter().all(|l| !l.starts_with("_exist")));
        assert!(property_lines(&json!({"long": "x".repeat(500)}))[0].chars().count() <= 201);
    }

    /// furnish-nix.sh's matching of store names ("<pname>-<version>[-<output>]") to packages.
    #[cfg(unix)]
    #[test]
    fn nix_store_names_match_packages() {
        let script = r#"
            check() { got=$(printf '%s\n' "$1" | nix_find $2) || got=none; [ "$got" = "$3" ] || echo "'$1' / $2: got '$got', want '$3'"; }
            check 'nixos jq-1.7.1-bin' jq 'nixos 1.7.1'
            check 'nixos jq-1.7.1-man' jq 'nixos 1.7.1'
            check 'nixos jq-1.7.1' jq 'nixos 1.7.1'
            check 'nixos git-lfs-3.6.1' git none
            check 'nixos git-2.47.2' git 'nixos 2.47.2'
            check 'nixos jquery' jq none
            check 'nixos hello' hello 'nixos ?'
            check 'home-manager kdenlive-25.08.1' "$(nix_guess kdePackages.kdenlive)" 'home-manager 25.08.1'
            check 'nixos python3.12-requests-2.32.3' "$(nix_guess python3Packages.requests)" 'nixos 2.32.3'
            check 'nixos python3.13-requests-toolbelt-1.0' "$(nix_guess python3Packages.requests)" none
            check 'nixos python3.12-requests-2.32.3' python3.12-requests 'nixos 2.32.3'
            check 'nixos neovim-unwrapped-0.11' neovim none
            check 'nixos vscode-1.99.0' vscode 'nixos 1.99.0'
            printf 'nixos fd-10\nhome-manager jq-1.8\n' | nix_find jq
        "#;
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("{}\n{script}", include_str!("../resources/unix/furnish-nix.sh")))
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "home-manager 1.8\n");
        assert!(out.stderr.is_empty(), "{}", String::from_utf8_lossy(&out.stderr));
    }
}
