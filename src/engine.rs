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
    dscresources::invoke_result::{SetResult, TestResult},
    progress::ProgressFormat,
};
use serde_json::Value;

/// Resources shipped inside the furnish binary: (file name, contents).
/// Same resource types on every OS, with OS-specific implementations.
#[cfg(not(windows))]
const BUNDLED_RESOURCES: &[(&str, &str)] = &[
    (
        "furnish-package.dsc.resource.json",
        include_str!("../resources/unix/furnish-package.dsc.resource.json"),
    ),
    (
        "furnish-package.dsc.resource.sh",
        include_str!("../resources/unix/furnish-package.dsc.resource.sh"),
    ),
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
    let _ = tracing_subscriber::registry().with(LogForwarder(Box::new(sink))).try_init();
}

struct LogForwarder(Box<dyn Fn(LogLine) + Send + Sync>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for LogForwarder {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        let meta = event.metadata();
        if !meta.target().starts_with("dsc_lib") {
            return;
        }
        let mut message = String::new();
        event.record(&mut MessageVisitor(&mut message));
        if let Some(line) = log_line(*meta.level(), &message) {
            (self.0)(line);
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
}

/// Write the bundled resources to `dir` and restrict DSC resource discovery to it.
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
    let mut path: Vec<PathBuf> = vec![dir.to_path_buf()];
    path.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    let stop_file = STOP_FILE.get_or_init(|| dir.join(".stop"));
    // SAFETY: called once at startup from main, before any threads are spawned.
    unsafe {
        env::set_var("DSC_RESOURCE_PATH", dir);
        env::set_var("PATH", env::join_paths(path)?);
        env::set_var("FURNISH_STOP_FILE", stop_file);
    }
    Ok(())
}

// DSC v3.3 reports resource failures as `Err`; `had_errors`/`messages` are never set.
pub fn status(doc: &Value) -> Result<Status> {
    let _guard = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    let result = configurator(doc)?.invoke_test()?;
    let mut states = Vec::new();
    flatten(&result.results, &mut states);
    Ok(classify(&states))
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
#[derive(Debug, Clone, serde::Serialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    Install,
    Remove,
    Change,
    Nothing,
    /// Its `condition` excludes it on this machine.
    Skipped,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Change {
    pub property: String,
    pub before: Value,
    pub after: Value,
}

/// Progress of an install/update/uninstall, reported while it runs.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "event", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Progress {
    /// The steps that will run, in order (same as `plan`).
    Planned { steps: Vec<Step> },
    Started { index: usize, at_ms: u64 },
    Finished { index: usize, at_ms: u64, duration_ms: u64, outcome: Outcome },
}

/// What a step actually did.
#[derive(Debug, Clone, serde::Serialize)]
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
    step_docs(doc, uninstall)?.iter().map(preview).collect()
}

/// Install (or uninstall) one resource at a time, reporting each step as it starts and ends.
/// furnish orchestrates the steps itself because DSC only reports per-resource progress on stderr.
/// Fails with [`Cancelled`] if the user aborted, force-stopped or refused the password dialog.
pub fn run(doc: &Value, uninstall: bool, mut report: impl FnMut(Progress)) -> Result<()> {
    let _guard = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
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
    resource["dependsOn"].as_array().into_iter().flatten().filter_map(|d| {
        let args = d.as_str()?.strip_prefix("[resourceId(")?.strip_suffix(")]")?;
        let (_, name) = args.split_once(',')?;
        Some(name.trim().trim_matches('\'').to_string())
    }).collect()
}

/// What-if for one single-resource document.
fn preview(doc: &Value) -> Result<Step> {
    let resource = &doc["resources"][0];
    let name = resource["name"].as_str().unwrap_or_default().to_string();
    let kind = resource["type"].as_str().unwrap_or_default().to_string();
    let mut configurator = configurator(doc)?;
    configurator.context.execution_type = ExecutionKind::WhatIf;
    let result = configurator.invoke_set(false)?;
    // A resource skipped by its condition produces no result at all.
    let Some(r) = result.results.first() else {
        let details = resource["condition"].as_str().map(|c| format!("Not on this system: {c}")).into_iter().collect();
        return Ok(Step { name, kind, action: Action::Skipped, details, changes: vec![] });
    };
    let details = r.metadata.as_ref()
        .and_then(|m| m.other.get("whatIf"))
        .and_then(Value::as_array)
        .map(|lines| lines.iter().filter_map(|l| l.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let (before, after) = match &r.result {
        SetResult::Resource(r) => (r.before_state.clone(), r.after_state.clone()),
        SetResult::Group(_) => (Value::Null, Value::Null),
    };
    Ok(Step { name, kind, action: action(&before, &after), details, changes: changes(&before, &after) })
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

fn configurator(doc: &Value) -> Result<Configurator> {
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
/// No states at all means every resource was skipped by its `condition`.
fn classify(states: &[(bool, Value)]) -> Status {
    if states.is_empty() {
        return Status::Unavailable;
    }
    let drifted: Vec<_> = states.iter().filter(|(ok, _)| !ok).map(|(_, actual)| actual).collect();
    if drifted.is_empty() {
        Status::Installed
    } else if drifted.iter().any(|a| matches!(&a["_exist"], Value::Bool(false)) || a["_exist"] == "false") {
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
        assert_eq!(classify(&[]), Status::Unavailable);
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
}
