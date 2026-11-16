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
    /// Start the desktop app at sign-in, in the tray, so updates run without anyone opening it.
    pub start_at_sign_in: bool,
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
    /// Commits of a collection's history to keep; 0 keeps all of them.
    pub checkout_depth: u32,
    /// Download file contents only when they are needed: those of earlier versions when furnish
    /// falls back to one, installer files (.msi, .deb, …) when an app using them is installed.
    pub blobs_on_demand: bool,
    /// What to do when installing or updating an app fails.
    pub fallback: Fallback,
    /// Only fall back to versions at most this many days older than the failing one; 0: any age.
    pub fallback_max_age_days: u32,
    /// Reports to the organization's furnish address (see `reports`): off, errors or all.
    pub reporting: crate::reports::Level,
    /// Include this computer's name in reports.
    pub report_hostname: bool,
    /// The user's values for the four settings above for single collections, keyed by collection.
    pub collections: std::collections::BTreeMap<String, Map<String, Value>>,
    /// The desktop app's language: the system's, or one the app has.
    pub language: Language,
    /// The language for a collection's texts when they aren't translated into the app's language
    /// (a BCP 47 tag such as `en` or `pt-BR`; see `locale`).
    pub fallback_language: String,
}

/// The languages of the desktop app (`Language` without `System`).
pub const UI_LANGUAGES: [&str; 8] = ["en", "de", "fr", "es", "it", "pl", "pt", "nl"];

/// The desktop app's language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Language {
    /// The system's language if the app has it, else English.
    System,
    En,
    De,
    Fr,
    Es,
    It,
    Pl,
    Pt,
    Nl,
}

/// What furnish does when installing or updating an app fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Fallback {
    /// Offer to install the previous version of the app.
    Ask,
    /// Install the previous version right away.
    Automatic,
    Off,
}

/// The settings that apply per collection.
pub const PER_COLLECTION: [&str; 4] = ["checkoutDepth", "blobsOnDemand", "fallback", "fallbackMaxAgeDays"];

/// The settings for one collection, in effect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionSettings {
    pub checkout_depth: u32,
    pub blobs_on_demand: bool,
    pub fallback: Fallback,
    pub fallback_max_age_days: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionEffective {
    pub settings: CollectionSettings,
    /// Setting names users can't change for this collection.
    pub locked: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            start_at_sign_in: true,
            update_on_launch: true,
            update_interval_minutes: 30,
            no_auto_update: Vec::new(),
            load_aware_updates: true,
            load_limits: crate::load::Limits::default(),
            parameters: Default::default(),
            checkout_depth: 0,
            blobs_on_demand: true,
            fallback: Fallback::Ask,
            fallback_max_age_days: 0,
            reporting: crate::reports::Level::Off,
            report_hostname: false,
            collections: Default::default(),
            language: Language::System,
            fallback_language: "en".into(),
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
    /// Values that were ignored because they aren't valid for their setting, and why: from the
    /// admin policy, an endpoint's description or the user's own file.
    pub warnings: Vec<String>,
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
    check_value(key, &value).with_context(|| format!("invalid value for '{key}'"))?;
    let mut choices = user_choices();
    choices.insert(key.to_string(), value);
    write_choices(&choices)
}

fn write_choices(choices: &Map<String, Value>) -> Result<()> {
    let dir = data_dir();
    fs::create_dir_all(&dir)?;
    let path = dir.join("settings.json");
    // Option values may be personal: only the user can read them.
    crate::auth::write_private(&path, (serde_json::to_string_pretty(choices)? + "\n").as_bytes()).with_context(|| format!("writing {}", path.display()))
}

/// The settings for one collection: the policy's value for the collection if it is locked; a
/// locked general value unless the policy sets one for the collection; else the user's value for
/// the collection, the policy's for the collection, the general value.
pub fn for_collection(name: &str) -> CollectionEffective {
    collection_merge(&crate::policy::load(), &user_choices(), name)
}

pub fn collection_merge(policy: &Policy, choices: &Map<String, Value>, name: &str) -> CollectionEffective {
    let general = merge(policy, choices);
    let defaults = serde_json::to_value(&general.settings).unwrap_or_default();
    let theirs = policy.collection_settings.get(name);
    let mine = general.settings.collections.get(name);
    let mut values = Map::new();
    let mut locked = Vec::new();
    for key in PER_COLLECTION {
        let policy_value = theirs.and_then(|t| t.get(key));
        // An invalid value counts as not set, so it doesn't spoil the collection's other settings.
        let valid = |v: &&Value| check_value(key, v).is_ok();
        let value = match policy_value {
            Some(e) if e.locked => Some(&e.value),
            None if general.locked.iter().any(|k| k == key) => None,
            _ => mine.and_then(|m| m.get(key)).filter(valid).or(policy_value.map(|e| &e.value)),
        }
        .filter(valid);
        if policy_value.is_some_and(|e| e.locked) || (policy_value.is_none() && general.locked.iter().any(|k| k == key)) {
            locked.push(key.to_string());
        }
        values.insert(key.to_string(), value.unwrap_or(&defaults[key]).clone());
    }
    let settings = serde_json::from_value(Value::Object(values)).unwrap_or_else(|err| {
        eprintln!("warning: ignoring invalid settings for collection '{name}': {err}");
        serde_json::from_value(defaults).expect("the general settings contain the collection settings")
    });
    CollectionEffective { settings, locked }
}

/// Change one setting for one collection; `null` removes the user's value.
pub fn set_for_collection(name: &str, key: &str, value: Value) -> Result<()> {
    if !PER_COLLECTION.contains(&key) {
        bail!("'{key}' isn't a setting per collection (those are {})", PER_COLLECTION.join(", "));
    }
    let policy = crate::policy::load();
    if collection_merge(&policy, &user_choices(), name).locked.iter().any(|k| k == key) {
        bail!("'{key}' is set by your organization for collection '{name}'");
    }
    let mut choices = user_choices();
    let mut collections = choices.get("collections").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut own = collections.get(name).and_then(Value::as_object).cloned().unwrap_or_default();
    if value.is_null() {
        own.remove(key);
    } else {
        check_value(key, &value).with_context(|| format!("invalid value for '{key}'"))?;
        own.insert(key.to_string(), value);
    }
    collections.insert(name.to_string(), own.into());
    choices.insert("collections".into(), collections.into());
    write_choices(&choices)
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

/// Defaults ← policy values ← user choices (unless locked). Each value counts on its own: one
/// that isn't valid for its setting is left out (see `Effective::warnings`), the others apply.
pub fn merge(policy: &Policy, choices: &Map<String, Value>) -> Effective {
    let Value::Object(mut merged) = serde_json::to_value(Settings::default()).expect("Settings serializes") else {
        unreachable!("Settings serializes to an object")
    };
    let mut warnings = policy.warnings.clone();
    for (key, entry) in &policy.settings {
        if !merged.contains_key(key) {
            warnings.push(format!("the admin policy names an unknown setting '{key}'"));
        } else if let Err(err) = check_value(key, &entry.value) {
            warnings.push(format!("ignoring the admin policy's value for '{key}': {err:#}"));
        } else {
            merged.insert(key.clone(), entry.value.clone());
        }
    }
    for (key, value) in choices {
        let locked = policy.settings.get(key).is_some_and(|e| e.locked);
        if !merged.contains_key(key) || locked {
            continue;
        }
        match check_value(key, value) {
            Ok(()) => {
                merged.insert(key.clone(), value.clone());
            }
            Err(err) => warnings.push(format!("ignoring your value for '{key}' in settings.json: {err:#}")),
        }
    }
    let settings = serde_json::from_value(Value::Object(merged)).unwrap_or_else(|err| {
        warnings.push(format!("ignoring invalid settings: {err}"));
        Settings::default()
    });
    Effective {
        settings,
        locked: policy.settings.iter().filter(|(_, e)| e.locked).map(|(k, _)| k.clone()).collect(),
        users_can_add_collections: policy.collections.users_can_add,
        warnings,
    }
}

/// Whether a value is valid for a setting, on its own (the other settings at their defaults).
pub fn check_value(key: &str, value: &Value) -> Result<()> {
    let Value::Object(mut all) = serde_json::to_value(Settings::default())? else {
        unreachable!("Settings serializes to an object")
    };
    if !all.contains_key(key) {
        bail!("unknown setting '{key}'");
    }
    all.insert(key.to_string(), value.clone());
    let settings: Settings = serde_json::from_value(Value::Object(all))?;
    if !crate::locale::valid_tag(&settings.fallback_language) {
        bail!("'{}' isn't a language tag such as en or pt-BR (fallbackLanguage)", settings.fallback_language);
    }
    Ok(())
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

    #[test]
    fn collection_settings_follow_policy_locks() {
        let p = policy(json!({
            "settings": {"fallback": {"value": "off", "locked": true}, "checkoutDepth": {"value": 50}},
            "collectionSettings": {"tools": {"fallback": {"value": "ask"}, "fallbackMaxAgeDays": {"value": 30, "locked": true}}}
        }));
        let choices = json!({"checkoutDepth": 10, "collections": {
            "tools": {"fallback": "automatic", "fallbackMaxAgeDays": 0, "checkoutDepth": 1},
            "other": {"fallback": "ask", "blobsOnDemand": false}
        }});
        let choices = choices.as_object().unwrap();
        // The policy sets fallback for "tools" without locking it: the user's value wins there.
        let tools = collection_merge(&p, choices, "tools");
        assert_eq!(tools.settings.fallback, Fallback::Automatic);
        assert_eq!(tools.settings.fallback_max_age_days, 30);
        assert_eq!(tools.settings.checkout_depth, 1);
        assert_eq!(tools.locked, ["fallbackMaxAgeDays"]);
        // Elsewhere the locked general value applies, whatever the user set for the collection.
        let other = collection_merge(&p, choices, "other");
        assert_eq!(other.settings.fallback, Fallback::Off);
        assert!(!other.settings.blobs_on_demand);
        assert_eq!(other.settings.checkout_depth, 10);
        assert_eq!(other.locked, ["fallback"]);
        // Defaults: blobless, whole history, ask.
        let none = collection_merge(&Policy::default(), &Map::new(), "x").settings;
        assert_eq!((none.checkout_depth, none.blobs_on_demand, none.fallback), (0, true, Fallback::Ask));
    }

    #[test]
    fn one_invalid_value_doesnt_spoil_the_others() {
        // As an endpoint once served it (issue #1): the policy's wrapped form inside a value.
        let p = policy(json!({"settings": {
            "reporting": {"value": {"value": "off"}},
            "updateIntervalMinutes": {"value": 60},
            "nonsense": {"value": 1}
        }}));
        let choices = json!({"parameters": {"tools": {"role": "dev"}}, "fallback": "sometimes", "checkoutDepth": 5});
        let e = merge(&p, choices.as_object().unwrap());
        assert_eq!(e.settings.update_interval_minutes, 60);
        assert_eq!(e.settings.reporting, crate::reports::Level::Off);
        assert_eq!(e.settings.parameters["tools"]["role"], "dev");
        assert_eq!(e.settings.checkout_depth, 5);
        assert_eq!(e.settings.fallback, Fallback::Ask);
        assert_eq!(e.warnings.len(), 3, "{:?}", e.warnings);
        assert!(e.warnings.iter().any(|w| w.contains("'reporting'") && w.contains("unknown variant")), "{:?}", e.warnings);
        assert!(e.warnings.iter().any(|w| w.contains("'fallback' in settings.json")), "{:?}", e.warnings);
        // Saving a value checks that value only.
        assert!(check_value("parameters", &json!({"tools": {"role": "admin"}})).is_ok());
        assert!(check_value("reporting", &json!({"value": "off"})).is_err());
        assert!(check_value("nonsense", &json!(1)).is_err());
        // Per collection, an invalid value counts as not set.
        let p = policy(json!({"collectionSettings": {"tools": {"fallback": {"value": "sometimes"}, "checkoutDepth": {"value": 3}}}}));
        let tools = collection_merge(&p, json!({"collections": {"tools": {"blobsOnDemand": "no"}}}).as_object().unwrap(), "tools").settings;
        assert_eq!((tools.fallback, tools.checkout_depth, tools.blobs_on_demand), (Fallback::Ask, 3, true));
    }

    #[test]
    fn start_at_sign_in_is_on_by_default_and_follows_the_policy() {
        assert!(Settings::default().start_at_sign_in);
        let off = json!({"startAtSignIn": false});
        assert!(!merge(&Policy::default(), off.as_object().unwrap()).settings.start_at_sign_in);
        // A policy default yields to the user; a locked value doesn't.
        let p = policy(json!({"settings": {"startAtSignIn": {"value": false}}}));
        assert!(!merge(&p, &Map::new()).settings.start_at_sign_in);
        assert!(merge(&p, json!({"startAtSignIn": true}).as_object().unwrap()).settings.start_at_sign_in);
        let p = policy(json!({"settings": {"startAtSignIn": {"value": true, "locked": true}}}));
        let e = merge(&p, off.as_object().unwrap());
        assert!(e.settings.start_at_sign_in);
        assert_eq!(e.locked, ["startAtSignIn"]);
    }

    #[test]
    fn language_defaults_to_system_and_can_be_locked() {
        assert_eq!(Settings::default().language, Language::System);
        let choices = json!({"language": "de"});
        let e = merge(&Policy::default(), choices.as_object().unwrap());
        assert_eq!(e.settings.language, Language::De);
        let p = policy(json!({"settings": {"language": {"value": "en", "locked": true}}}));
        let e = merge(&p, choices.as_object().unwrap());
        assert_eq!(e.settings.language, Language::En);
        assert_eq!(e.locked, ["language"]);
        // Unknown languages are rejected.
        assert!(check_value("language", &json!("xx")).is_err());
    }

    #[test]
    fn fallback_language_defaults_to_english_takes_any_tag_and_can_be_locked() {
        assert_eq!(Settings::default().fallback_language, "en");
        let e = merge(&Policy::default(), json!({"fallbackLanguage": "sv"}).as_object().unwrap());
        assert_eq!(e.settings.fallback_language, "sv");
        let p = policy(json!({"settings": {"fallbackLanguage": {"value": "de", "locked": true}}}));
        let e = merge(&p, json!({"fallbackLanguage": "fr"}).as_object().unwrap());
        assert_eq!(e.settings.fallback_language, "de");
        assert_eq!(e.locked, ["fallbackLanguage"]);
        assert!(check_value("fallbackLanguage", &json!("not a tag")).is_err());
    }
}
