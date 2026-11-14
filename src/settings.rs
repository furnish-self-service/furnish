//! Settings: built-in defaults, overridden by the admin policy (see `policy`), overridden by the
//! user's own choices in `settings.json` in the data directory, except where the policy locks a
//! setting. The user's file only holds what the user changed, so policy defaults keep applying.

use std::fs;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{catalog::data_dir, policy::Policy};

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
    /// The user's option values (see `options`), keyed by collection or `collection/app`.
    pub parameters: std::collections::BTreeMap<String, std::collections::BTreeMap<String, Value>>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            update_on_launch: true,
            update_interval_minutes: 30,
            no_auto_update: Vec::new(),
            load_aware_updates: true,
            load_limits: crate::load::Limits::default(),
            parameters: Default::default(),
        }
    }
}

/// The settings in effect and what the admin policy restricts.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Effective {
    pub settings: Settings,
    /// Setting names users can't change.
    pub locked: Vec<String>,
    /// From the policy: whether users may add collections.
    pub users_can_add_collections: bool,
}

pub fn load() -> Settings {
    effective().settings
}

pub fn effective() -> Effective {
    merge(&crate::policy::load(), &user_choices())
}

/// Change one setting for this user. Fails if the policy locks it, or the name or value is wrong.
pub fn set(key: &str, value: Value) -> Result<()> {
    let policy = crate::policy::load();
    if !serde_json::to_value(Settings::default())?.as_object().is_some_and(|s| s.contains_key(key)) {
        bail!("unknown setting '{key}'");
    }
    if policy.settings.get(key).is_some_and(|e| e.locked) {
        bail!("'{key}' is set by your organization");
    }
    let mut choices = user_choices();
    choices.insert(key.to_string(), value);
    strict_merge(&policy, &choices).with_context(|| format!("invalid value for '{key}'"))?;
    let dir = data_dir();
    fs::create_dir_all(&dir)?;
    let path = dir.join("settings.json");
    fs::write(&path, serde_json::to_string_pretty(&choices)? + "\n").with_context(|| format!("writing {}", path.display()))
}

/// The user's own choices (only the settings they changed).
fn user_choices() -> Map<String, Value> {
    let path = data_dir().join("settings.json");
    let Ok(text) = fs::read_to_string(&path) else {
        return Map::new();
    };
    serde_json::from_str(&text).unwrap_or_else(|err| {
        eprintln!("warning: ignoring {}: {err}", path.display());
        Map::new()
    })
}

/// Defaults ← policy values ← user choices (unless locked). Invalid values fall back to defaults.
pub fn merge(policy: &Policy, choices: &Map<String, Value>) -> Effective {
    let settings = strict_merge(policy, choices).unwrap_or_else(|err| {
        eprintln!("warning: ignoring invalid settings: {err:#}");
        Settings::default()
    });
    Effective {
        settings,
        locked: policy.settings.iter().filter(|(_, e)| e.locked).map(|(k, _)| k.clone()).collect(),
        users_can_add_collections: policy.collections.users_can_add,
    }
}

fn strict_merge(policy: &Policy, choices: &Map<String, Value>) -> Result<Settings> {
    let Value::Object(mut merged) = serde_json::to_value(Settings::default())? else {
        unreachable!("Settings serializes to an object")
    };
    for (key, entry) in &policy.settings {
        if merged.contains_key(key) {
            merged.insert(key.clone(), entry.value.clone());
        } else {
            eprintln!("warning: the admin policy names an unknown setting '{key}'");
        }
    }
    for (key, value) in choices {
        let locked = policy.settings.get(key).is_some_and(|e| e.locked);
        if merged.contains_key(key) && !locked {
            merged.insert(key.clone(), value.clone());
        }
    }
    Ok(serde_json::from_value(Value::Object(merged))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn policy(v: Value) -> Policy {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn missing_fields_use_defaults() {
        let s: Settings = serde_json::from_str(r#"{"updateIntervalMinutes": 0}"#).unwrap();
        assert_eq!(s, Settings { update_interval_minutes: 0, ..Settings::default() });
        assert!(s.load_aware_updates);
    }

    #[test]
    fn policy_defaults_yield_to_users_but_locks_dont() {
        let p = policy(json!({"settings": {
            "updateIntervalMinutes": {"value": 60},
            "loadAwareUpdates": {"value": false, "locked": true}
        }}));
        // No user choice: the policy values apply.
        let e = merge(&p, &Map::new());
        assert_eq!(e.settings.update_interval_minutes, 60);
        assert!(!e.settings.load_aware_updates);
        assert_eq!(e.locked, ["loadAwareUpdates"]);
        // A user choice wins over a policy default, not over a locked value.
        let choices = json!({"updateIntervalMinutes": 15, "loadAwareUpdates": true});
        let e = merge(&p, choices.as_object().unwrap());
        assert_eq!(e.settings.update_interval_minutes, 15);
        assert!(!e.settings.load_aware_updates);
    }
}
