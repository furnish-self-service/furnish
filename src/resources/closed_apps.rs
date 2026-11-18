//! The executable basenames that must be closed before changing a settings file.
use std::ffi::OsStr;

use anyhow::{Result, ensure};
use serde::Deserialize;
use serde_json::Value;
use sysinfo::{ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind};

#[derive(Default, Deserialize)]
#[serde(try_from = "Value")]
pub(super) struct ClosedApps(Vec<String>);

fn validate_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty() && name.trim() == name && !matches!(name, "." | "..")
            && !name.contains(['/', '\\', ':', '*', '?', '[', ']']) && !name.chars().any(char::is_control),
        "requiresClosed expects executable basenames; invalid name {name:?}"
    );
    Ok(())
}

impl TryFrom<Value> for ClosedApps {
    type Error = anyhow::Error;
    fn try_from(value: Value) -> Result<Self> {
        let names: Vec<String> = serde_json::from_value(value)
            .map_err(|_| anyhow::anyhow!("requiresClosed expects an array of executable basenames"))?;
        ensure!(!names.is_empty(), "requiresClosed needs at least one executable basename");
        for name in &names {
            validate_name(name)?;
        }
        Ok(Self(names))
    }
}

/// A property or list item may be a DSC expression. Check the remaining literal names
/// even when another property of the same resource depends on an expression.
pub(super) fn check_property(property: &Value) -> Result<()> {
    let expression = |v: &Value| v.as_str().is_some_and(|s| s.starts_with('[') && !s.starts_with("[[") && s.ends_with(']'));
    if expression(property) {
        return Ok(());
    }
    let names = property.as_array().filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("requiresClosed needs a nonempty array of executable basenames"))?;
    for name in names.iter().filter(|v| !expression(v)) {
        validate_name(name.as_str().ok_or_else(|| anyhow::anyhow!("requiresClosed expects executable basenames"))?)?;
    }
    Ok(())
}

impl ClosedApps {
    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(super) fn ensure_closed(&self) -> Result<()> {
        let running = running(&self.0)?;
        ensure!(running.is_empty(), "Close {}, then retry this change", running.join(", "));
        Ok(())
    }
}

/// Match only the requested executable basenames against one fresh process snapshot.
pub(crate) fn running(names: &[String]) -> Result<Vec<String>> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let mut system = System::new();
    let refreshed = system.refresh_processes_specifics(
        ProcessesToUpdate::All, true, ProcessRefreshKind::nothing().without_tasks().with_exe(UpdateKind::Always),
    );
    ensure!(refreshed > 0, "could not check running applications; retry this change when the process list is available");
    let running = names.iter().filter(|wanted| {
        system.processes().values().any(|p| {
            // A zombie can no longer write. On Linux the kernel process name can be
            // truncated or renamed, so prefer the executable path whenever available.
            let basename = p.exe().and_then(|path| path.file_name()).unwrap_or_else(|| p.name());
            p.status() != ProcessStatus::Zombie && matches_name(basename, wanted)
        })
    }).cloned().collect();
    Ok(running)
}

#[cfg(not(windows))]
fn matches_name(actual: &OsStr, wanted: &str) -> bool {
    actual == wanted
}

#[cfg(windows)]
fn matches_name(actual: &OsStr, wanted: &str) -> bool {
    let Some(actual) = actual.to_str() else { return false };
    let actual = actual.to_lowercase();
    let wanted = wanted.to_lowercase();
    actual.strip_suffix(".exe").unwrap_or(&actual) == wanted.strip_suffix(".exe").unwrap_or(&wanted)
}
