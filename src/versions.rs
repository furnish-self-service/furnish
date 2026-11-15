//! Which version of an app furnish installed, for falling back after a failure (see the
//! `fallback` setting). A version is a commit of the collection. Kept in `versions.json` in the
//! data directory of whoever installs, like `installed.json`:
//!
//! - `good`: the last commit an app installed from without error.
//! - `held`: the app runs an earlier version because the current one failed; it stays there
//!   while the collection's file for the app is still the one that failed (`failed`, a blob id).

use std::{collections::BTreeMap, fs, path::Path};

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Record {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub good: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held: Option<Held>,
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
    records.insert(app_key.to_string(), Record { good: Some(commit.to_string()), held });
    write(dir, &records)
}

pub fn forget(dir: &Path, app_key: &str) -> Result<()> {
    let mut records = all(dir);
    if records.remove(app_key).is_some() {
        write(dir, &records)?;
    }
    Ok(())
}
