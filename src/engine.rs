//! Thin wrapper around the embedded DSC engine. The only module that touches dsc-lib,
//! so upstream API churn stays contained here.

use std::{env, fs, path::Path, path::PathBuf};

use anyhow::Result;
use dsc_lib::{
    configure::{Configurator, config_result::ResourceTestResult},
    dscresources::invoke_result::TestResult,
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

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
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
    // SAFETY: called once at startup from main, before any threads are spawned.
    unsafe {
        env::set_var("DSC_RESOURCE_PATH", dir);
        env::set_var("PATH", env::join_paths(path)?);
    }
    Ok(())
}

// DSC v3.3 reports resource failures as `Err`; `had_errors`/`messages` are never set.
pub fn status(doc: &Value) -> Result<Status> {
    let result = configurator(doc)?.invoke_test()?;
    let mut states = Vec::new();
    flatten(&result.results, &mut states);
    Ok(classify(&states))
}

pub fn apply(doc: &Value) -> Result<()> {
    configurator(doc)?.invoke_set(false)?;
    Ok(())
}

/// Apply the document with every resource flipped to `_exist: false`.
// ponytail: assumes every resource of an app supports `_exist`; per-app uninstall docs if not.
pub fn remove(doc: &Value) -> Result<()> {
    let mut doc = doc.clone();
    if let Some(resources) = doc["resources"].as_array_mut() {
        resources.reverse();
        for resource in resources {
            resource["properties"]["_exist"] = Value::Bool(false);
        }
    }
    apply(&doc)
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
}
