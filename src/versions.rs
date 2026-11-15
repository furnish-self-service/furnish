//! Which version of an app furnish installed, for falling back after a failure (see the
//! `fallback` setting). A version is a commit of the collection. Kept in `versions.json` in the
//! data directory of whoever installs, like `installed.json`:
//!
//! - `good`: the last commit an app installed from without error.
//! - `held`: the app runs an earlier version because the current one failed; it stays there
//!   while the collection's file for the app is still the one that failed (`failed`, a blob id).
//! - `failed`: installing the collection's file for the app (a blob id) failed here. Only then
//!   may the app go back to an earlier version (`catalog::App::install_previous_with`).

use std::{collections::BTreeMap, fs, path::Path};

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Record {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub good: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held: Option<Held>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed: Option<String>,
}

impl Record {
    /// Whether installing this file of the app (a blob id) failed here.
    pub fn has_failed(&self, blob: &str) -> bool {
        self.failed.as_deref() == Some(blob) || self.held.as_ref().is_some_and(|h| h.failed == blob)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Held {
    pub commit: String,
    pub failed: String,
}

pub fn all(dir: &Path) -> BTreeMap<String, Record> {
    fs::read_to_string(dir.join("versions.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn write(dir: &Path, all: &BTreeMap<String, Record>) -> Result<()> {
    fs::create_dir_all(dir)?;
    fs::write(dir.join("versions.json"), serde_json::to_string_pretty(all)? + "\n")?;
    Ok(())
}

pub fn get(dir: &Path, app_key: &str) -> Record {
    all(dir).remove(app_key).unwrap_or_default()
}

/// The app installed from `commit`; `held` if that is an earlier version than the collection's.
pub fn installed(dir: &Path, app_key: &str, commit: &str, held: Option<Held>) -> Result<()> {
    let mut records = all(dir);
    records.insert(app_key.to_string(), Record { good: Some(commit.to_string()), held, failed: None });
    write(dir, &records)?;
    prune(dir, &records);
    Ok(())
}

/// Installing the app's file `blob` failed.
pub fn failed(dir: &Path, app_key: &str, blob: &str) -> Result<()> {
    let mut records = all(dir);
    records.entry(app_key.to_string()).or_default().failed = Some(blob.to_string());
    write(dir, &records)
}

pub fn forget(dir: &Path, app_key: &str) -> Result<()> {
    let mut records = all(dir);
    if records.remove(app_key).is_some() {
        write(dir, &records)?;
        prune(dir, &records);
    }
    Ok(())
}

/// Delete the files of earlier versions (`versions/<collection>/<commit>/`, see
/// `catalog::version_dir`) that no app is held at anymore.
fn prune(dir: &Path, records: &BTreeMap<String, Record>) {
    let held: Vec<(String, String)> = records
        .iter()
        .filter_map(|(key, r)| Some((key.split('/').next()?.to_string(), r.held.as_ref()?.commit.clone())))
        .collect();
    let Ok(collections) = fs::read_dir(dir.join("versions")) else { return };
    for collection in collections.flatten() {
        let name = collection.file_name().to_string_lossy().into_owned();
        for version in fs::read_dir(collection.path()).into_iter().flatten().flatten() {
            let commit = version.file_name().to_string_lossy().into_owned();
            if !held.iter().any(|(c, h)| *c == name && *h == commit) {
                let _ = fs::remove_dir_all(version.path());
            }
        }
        let _ = fs::remove_dir(collection.path()); // only if empty
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_is_recorded_until_an_install_succeeds() {
        let dir = std::env::temp_dir().join(format!("furnish-versions-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        installed(&dir, "c/app", "c1", None).unwrap();
        assert!(!get(&dir, "c/app").has_failed("b2"));
        failed(&dir, "c/app", "b2").unwrap();
        let record = get(&dir, "c/app");
        assert_eq!(record.good.as_deref(), Some("c1")); // kept: the version to fall back to
        assert!(record.has_failed("b2") && !record.has_failed("b1"));
        installed(&dir, "c/app", "c1", Some(Held { commit: "c1".into(), failed: "b2".into() })).unwrap();
        assert!(get(&dir, "c/app").has_failed("b2")); // held: still the one that failed
        installed(&dir, "c/app", "c3", None).unwrap();
        assert!(!get(&dir, "c/app").has_failed("b2"));
        let _ = fs::remove_dir_all(&dir);
    }
}
