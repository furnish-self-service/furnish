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
    dscresources::invoke_result::{SetResult, TestResult},
    progress::ProgressFormat,
    types::{FullyQualifiedTypeName, TypeNameFilter},
};
use serde_json::Value;

/// Resources shipped inside the furnish binary: (file name, contents).
/// Same resource types on every OS, with OS-specific implementations.
#[cfg(not(windows))]
const BUNDLED_RESOURCES: &[(&str, &str)] = &[
    ("furnish-common.sh", include_str!("../resources/unix/furnish-common.sh")),
    ("furnish-package.dsc.resource.json", include_str!("../resources/unix/furnish-package.dsc.resource.json")),
    ("furnish-package.dsc.resource.sh", include_str!("../resources/unix/furnish-package.dsc.resource.sh")),
    ("furnish-installer.dsc.resource.json", include_str!("../resources/unix/furnish-installer.dsc.resource.json")),
    ("furnish-installer.dsc.resource.sh", include_str!("../resources/unix/furnish-installer.dsc.resource.sh")),
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
}

/// Resources report a missing tool as a plain stderr line "furnish: needs <what>" (exit 11).
pub fn missing_reason(error: &str) -> Option<String> {
    let rest = &error[error.find("furnish: needs ")? + "furnish: needs ".len()..];
    Some(rest.lines().next().unwrap_or_default().trim().to_string())
}

/// Where furnish wrote its own resources (see `init`).
static BUNDLED_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Write the bundled resources to `dir` and point DSC's resource discovery at them, then at the
/// folders DSC normally searches (see `search_dirs`).
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
    let _ = BUNDLED_DIR.set(dir.to_path_buf());
    let path: Vec<PathBuf> = env::split_paths(&env::var_os("PATH").unwrap_or_default()).collect();
    let in_service = crate::service::in_service();
    let mut dirs = vec![dir.to_path_buf()];
    dirs.extend(search_dirs(&path, in_service, crate::service::admin_only));
    let dirs = env::join_paths(dirs)?;
    let stop_file = STOP_FILE.get_or_init(|| dir.join(".stop"));
    // SAFETY: called once at startup from main, before any threads are spawned.
    unsafe {
        // DSC searches DSC_RESOURCE_PATH instead of PATH, and finds resources' programs on PATH,
        // then in the resource's folder. The same list for both, furnish's own resources first.
        env::set_var("DSC_RESOURCE_PATH", &dirs);
        env::set_var("PATH", &dirs);
        env::set_var("FURNISH_STOP_FILE", stop_file);
    }
    Ok(())
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
    let Some(doc) = applicable(doc) else {
        return Ok(Check { status: Status::Unavailable, reason: None });
    };
    let result = match configurator(&doc).and_then(|mut c| Ok(c.invoke_test()?)) {
        Ok(result) => result,
        Err(e) => {
            let reason = missing_resource(&e).or_else(|| missing_reason(&format!("{e:#}"))).ok_or(e)?;
            return Ok(Check { status: Status::Blocked, reason: Some(reason) });
        }
    };
    let mut states = Vec::new();
    flatten(&result.results, &mut states);
    Ok(Check { status: classify(&states), reason: None })
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
/// A condition furnish can't evaluate is left to DSC.
fn applies_here(resource: &Value, doc: &Value) -> bool {
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

/// Uninstall an app: every resource set to `_exist: false`, in reverse install order.
// ponytail: assumes every resource of an app supports `_exist`; per-app uninstall docs if not.
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
    /// Its `condition` excludes it on this machine.
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
    step_docs(doc, uninstall)?.iter().map(preview).collect()
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
    let steps = docs.iter().map(preview).collect::<Result<Vec<_>>>()?;
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
        let result = configurator(step_doc).and_then(|mut c| Ok(c.invoke_set(false)?));
        let finished = now_ms();
        let outcome = match &result {
            Ok(r) => match r.results.first().map(|r| &r.result) {
                Some(SetResult::Resource(r)) => Outcome {
                    action: action(&r.before_state, &r.after_state),
                    changes: changes(&r.before_state, &r.after_state),
                    error: None,
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
        report(Progress::Finished { index, at_ms: finished, duration_ms: finished - started, outcome });
        if cancelled {
            return Err(Cancelled.into());
        }
        result?;
    }
    Ok(())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// One single-resource document per step, in run order. Uninstall flips `_exist` and runs the
/// install order backwards, so dependents go before what they depend on.
fn step_docs(doc: &Value, uninstall: bool) -> Result<Vec<Value>> {
    let resources = doc["resources"].as_array().cloned().unwrap_or_default();
    let mut order = invocation_order(&resources)?;
    if uninstall {
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
            if uninstall {
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

/// What-if for one single-resource document.
fn preview(doc: &Value) -> Result<Step> {
    let resource = &doc["resources"][0];
    let name = resource["name"].as_str().unwrap_or_default().to_string();
    let kind = resource["type"].as_str().unwrap_or_default().to_string();
    let skipped = || {
        let details = resource["condition"].as_str().map(|c| format!("Not on this system: {c}")).into_iter().collect();
        Step { name: name.clone(), kind: kind.clone(), action: Action::Skipped, details, changes: vec![] }
    };
    if !applies_here(resource, doc) {
        return Ok(skipped());
    }
    let mut configurator = configurator(doc)?;
    configurator.context.execution_type = ExecutionKind::WhatIf;
    let result = configurator.invoke_set(false)?;
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
    Ok(Step { name, kind, action: action(&before, &after), details, changes: changes(&before, &after) })
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
    let mut doc = doc.clone();
    // DSC would discover resources again while running, after `ensure_own_resources` checked
    // them; furnish skips resources by their condition itself (see `applicable`).
    if let Some(directives) = doc["directives"].as_object_mut() {
        directives.remove("resourceDiscovery");
    }
    let mut configurator = Configurator::new(&doc.to_string(), ProgressFormat::None)?;
    configurator.set_context(None)?;
    Ok(configurator)
}

/// Collect (in_desired_state, actual_state) of every resource, descending into groups.
fn flatten(results: &[ResourceTestResult], out: &mut Vec<(bool, Value)>) {
    for result in results {
        match &result.result {
            TestResult::Resource(r) => out.push((r.in_desired_state, r.actual_state.clone())),
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
        assert!(crate::service::admin_only(Path::new("/")));
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
    fn other_resources_preview_their_properties() {
        let lines = property_lines(&json!({"keyPath": "HKLM\\Software\\Acme", "valueData": {"String": "x"}, "_exist": true, "script": "a\nb"}));
        assert!(lines.contains(&"keyPath: HKLM\\Software\\Acme".to_string()), "{lines:?}");
        assert!(lines.contains(&r#"valueData: {"String":"x"}"#.to_string()), "{lines:?}");
        assert!(lines.contains(&"script: a ⏎ b".to_string()), "{lines:?}");
        assert!(lines.iter().all(|l| !l.starts_with("_exist")));
        assert!(property_lines(&json!({"long": "x".repeat(500)}))[0].chars().count() <= 201);
    }
}
