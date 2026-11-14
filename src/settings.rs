//! User settings, stored as `settings.json` in the data directory. Admins can deploy the file;
//! missing fields fall back to the defaults.

use std::fs;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::catalog::data_dir;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Pull all collections when the desktop app starts.
    pub update_on_launch: bool,
    /// Pull all collections this often while the desktop app runs; 0 turns it off.
    pub update_interval_minutes: u32,
    /// Apps (`collection/app`) whose updates are not installed automatically.
    pub no_auto_update: Vec<String>,
    /// Postpone automatic updates while the computer is busy (5-minute averages over `load_limits`).
    pub load_aware_updates: bool,
    pub load_limits: crate::load::Limits,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            update_on_launch: true,
            update_interval_minutes: 30,
            no_auto_update: Vec::new(),
            load_aware_updates: true,
            load_limits: crate::load::Limits::default(),
        }
    }
}

/// The saved settings, or the defaults if there are none. A broken file is reported, not fatal.
pub fn load() -> Settings {
    let path = data_dir().join("settings.json");
    let Ok(text) = fs::read_to_string(&path) else {
        return Settings::default();
    };
    serde_json::from_str(&text).unwrap_or_else(|err| {
        eprintln!("warning: ignoring {}: {err}", path.display());
        Settings::default()
    })
}

pub fn save(settings: &Settings) -> Result<()> {
    let dir = data_dir();
    fs::create_dir_all(&dir)?;
    let path = dir.join("settings.json");
    fs::write(&path, serde_json::to_string_pretty(settings)? + "\n").with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_use_defaults() {
        let s: Settings = serde_json::from_str(r#"{"updateIntervalMinutes": 0}"#).unwrap();
        assert_eq!(s, Settings { update_interval_minutes: 0, ..Settings::default() });
        assert!(s.load_aware_updates);
    }
}
