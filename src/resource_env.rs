//! The settings in effect and what furnish detected, as environment variables for every resource
//! furnish runs (its own and a collection's):
//!
//! - `FURNISH_CONFIG__<KEY>…`: every setting with the value that applies, as `furnish settings`
//!   shows it (admin policy, the user's choice, the furnish address's default, built-in default);
//! - `FURNISH_DETECTED__<KEY>…`: facts that aren't settings, e.g. where the NixOS configuration
//!   is (`nixos::detected`).
//!
//! Levels are separated by `__`, the words of a camelCase key by `_`, upper case:
//! `osSpecific.nixos.flakeAttribute` → `FURNISH_CONFIG__OS_SPECIFIC__NIXOS__FLAKE_ATTRIBUTE`.
//! Values: strings as they are, `true`/`false`, numbers, arrays and maps as compact JSON, empty
//! when unknown (unset).
//!
//! The list comes from `Settings`, so a new setting is passed on without more code. Not passed:
//! `NOT_EXPORTED`. Secrets (sign-in tokens, passwords) aren't settings; a test fails for a setting
//! whose name sounds like one unless `NOT_SECRET` names it.
//!
//! In the furnish service, the settings come from the admin policy and the defaults only, never
//! from a user's settings file: resources run as root there.

use std::sync::Mutex;

use serde_json::{Map, Value};

pub const CONFIG: &str = "FURNISH_CONFIG";
pub const DETECTED: &str = "FURNISH_DETECTED";

/// Settings not passed: option values may be personal, and each app gets its own as DSC parameters.
pub const NOT_EXPORTED: [&str; 1] = ["parameters"];

/// Settings whose name contains token, password, secret or credential but which aren't secrets.
pub const NOT_SECRET: [&str; 0] = [];

/// `osSpecific.nixos` → `OS_SPECIFIC__NIXOS`, after the prefix and `__`.
pub fn encode(prefix: &str, path: &[&str]) -> String {
    let mut name = prefix.to_string();
    for key in path {
        name.push_str("__");
        for c in key.chars() {
            if c.is_ascii_uppercase() {
                name.push('_');
            }
            name.push(c.to_ascii_uppercase());
        }
    }
    name
}

/// The prefix and the keys of a variable name (`encode` backwards).
pub fn decode(name: &str) -> Option<(String, Vec<String>)> {
    let mut parts = name.split("__");
    let prefix = parts.next()?.to_string();
    let keys: Vec<String> = parts
        .map(|level| {
            let mut key = String::new();
            for (i, word) in level.split('_').enumerate() {
                let word = word.to_ascii_lowercase();
                if i == 0 {
                    key.push_str(&word);
                } else {
                    let mut c = word.chars();
                    key.extend(c.next().map(|f| f.to_ascii_uppercase()));
                    key.push_str(c.as_str());
                }
            }
            key
        })
        .collect();
    Some((prefix, keys))
}

/// The paths of the values in `schema`: a map with keys is a level; an empty map (keyed by
/// collection or app), a list or a plain value is one value.
fn leaves(schema: &Value, path: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
    match schema.as_object() {
        Some(o) if !o.is_empty() => {
            for (k, v) in o {
                path.push(k.clone());
                leaves(v, path, out);
                path.pop();
            }
        }
        _ => out.push(path.clone()),
    }
}

/// The paths of every setting passed on.
pub fn setting_paths() -> Vec<Vec<String>> {
    let mut schema = serde_json::to_value(crate::settings::Settings::default()).expect("Settings serializes");
    for key in NOT_EXPORTED {
        schema.as_object_mut().expect("Settings is a map").remove(key);
    }
    let mut out = Vec::new();
    leaves(&schema, &mut Vec::new(), &mut out);
    out
}

fn text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        // true/false, numbers, and arrays and maps as compact JSON.
        _ => v.to_string(),
    }
}

fn at<'a>(v: &'a Value, path: &[String]) -> &'a Value {
    path.iter().fold(v, |v, k| &v[k.as_str()])
}

/// The variables for these settings (as `Settings` serializes) and detected facts.
pub fn vars(settings: &Value, detected: &Value) -> Vec<(String, String)> {
    let named = |prefix, path: &[String]| encode(prefix, &path.iter().map(String::as_str).collect::<Vec<_>>());
    let mut out: Vec<(String, String)> = setting_paths().iter().map(|p| (named(CONFIG, p), text(at(settings, p)))).collect();
    let mut found = Vec::new();
    leaves(detected, &mut Vec::new(), &mut found);
    out.extend(found.iter().map(|p| (named(DETECTED, p), text(at(detected, p)))));
    out
}

/// The variables for this process: in the service without the user's settings.
pub fn current() -> Vec<(String, String)> {
    let choices = if crate::service::in_service() { Map::new() } else { crate::settings::user_choices() };
    let effective = crate::settings::merge(&crate::policy::load(), &choices);
    let settings = serde_json::to_value(effective.settings).unwrap_or_default();
    vars(&settings, &crate::nixos::detected_here())
}

/// What the process environment holds now.
static SET: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// Set every variable, also the empty ones, so later changes (`refresh`) only replace values and
/// never add or remove names.
///
/// # Safety
/// Only while no other thread runs (like `engine::init`).
pub unsafe fn init() {
    let vars = current();
    for (k, v) in &vars {
        // SAFETY: the caller guarantees a single thread.
        unsafe { std::env::set_var(k, v) };
    }
    *SET.lock().unwrap_or_else(|e| e.into_inner()) = vars;
}

/// Before a DSC run in the desktop app: pass settings changed since the last run. Only values
/// change, names stay (see `init`), so glibc replaces an entry of the environment in place
/// instead of moving it while another thread may read it; glibc also keeps the old strings.
/// The service (a static build, maybe not glibc) doesn't refresh: the NixOS module restarts it
/// when the policy changes, and it ignores users' settings anyway.
pub fn refresh() {
    if crate::service::in_service() {
        return;
    }
    let vars = current();
    let mut set = SET.lock().unwrap_or_else(|e| e.into_inner());
    if *set == vars || set.is_empty() {
        return;
    }
    for ((k, old), (_, new)) in set.iter().zip(&vars).filter(|((k, _), (k2, _))| k == k2) {
        if old != new {
            // SAFETY: names don't change after `init` (see above); std's own environment reads
            // take the same lock as set_var.
            unsafe { std::env::set_var(k, new) };
        }
    }
    *set = vars;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Every setting and detected fact, with its prefix.
    fn all_paths() -> Vec<(&'static str, Vec<String>)> {
        let mut found = Vec::new();
        leaves(&crate::nixos::detected(None), &mut Vec::new(), &mut found);
        setting_paths().into_iter().map(|p| (CONFIG, p)).chain(found.into_iter().map(|p| (DETECTED, p))).collect()
    }

    #[test]
    fn names_follow_the_scheme() {
        assert_eq!(encode(CONFIG, &["updateOnLaunch"]), "FURNISH_CONFIG__UPDATE_ON_LAUNCH");
        assert_eq!(encode(CONFIG, &["osSpecific", "nixos", "flakeAttribute"]), "FURNISH_CONFIG__OS_SPECIFIC__NIXOS__FLAKE_ATTRIBUTE");
        assert_eq!(encode(DETECTED, &["osSpecific", "nixos", "configurationSource"]), "FURNISH_DETECTED__OS_SPECIFIC__NIXOS__CONFIGURATION_SOURCE");
    }

    #[test]
    fn every_name_decodes_back_and_none_collide() {
        let mut names = std::collections::BTreeSet::new();
        for (prefix, path) in all_paths() {
            for key in &path {
                // Keys that would break the scheme: underscores, leading capitals, nothing.
                assert!(key.chars().next().is_some_and(|c| c.is_ascii_lowercase()), "{key}");
                assert!(key.chars().all(|c| c.is_ascii_alphanumeric()), "{key}");
                assert!(!key.chars().zip(key.chars().skip(1)).any(|(a, b)| a.is_ascii_uppercase() && b.is_ascii_uppercase()), "{key}");
            }
            let refs: Vec<&str> = path.iter().map(String::as_str).collect();
            let name = encode(prefix, &refs);
            assert!(!name.ends_with('_') && !name.contains("___"), "{name}");
            assert_eq!(decode(&name), Some((prefix.to_string(), path.clone())), "{name}");
            assert!(names.insert(name.clone()), "two settings make {name}");
        }
        assert!(names.contains("FURNISH_CONFIG__OS_SPECIFIC__NIXOS__CONFIGURATION"));
        assert!(names.contains("FURNISH_DETECTED__OS_SPECIFIC__NIXOS__CONFIGURATION_OWNER"));
        assert!(names.contains("FURNISH_CONFIG__LOAD_LIMITS__CPU_PERCENT"), "{names:?}");
    }

    #[test]
    fn no_secrets_among_the_settings() {
        for (_, path) in all_paths() {
            let joined = path.join(".");
            let lower = joined.to_lowercase();
            let secret = ["token", "password", "secret", "credential"].iter().any(|w| lower.contains(w));
            assert!(!secret || NOT_SECRET.contains(&joined.as_str()), "'{joined}' sounds like a secret: don't pass it to resources, or add it to NOT_SECRET");
        }
    }

    #[test]
    fn values_are_the_effective_settings() {
        let policy: crate::policy::Policy = serde_json::from_value(json!({
            "settings": {"updateIntervalMinutes": {"value": 60, "locked": true}},
            "osSpecific": {"nixos": {"flakeAttribute": {"value": "desk", "locked": true}}}
        }))
        .unwrap();
        let choices = json!({"osSpecific": {"nixos": {"configuration": "/home/alice/cfg", "flakeAttribute": "mine"}},
                             "noAutoUpdate": ["it/vpn"], "parameters": {"it": {"secretish": "x"}}});
        let e = crate::settings::merge(&policy, choices.as_object().unwrap());
        let detected = json!({"osSpecific": {"nixos": {"configuration": "/home/alice/cfg", "configurationSource": "setting", "homeManagerConfiguration": null}}});
        let vars: std::collections::BTreeMap<_, _> = vars(&serde_json::to_value(e.settings).unwrap(), &detected).into_iter().collect();
        assert_eq!(vars["FURNISH_CONFIG__UPDATE_INTERVAL_MINUTES"], "60");
        assert_eq!(vars["FURNISH_CONFIG__UPDATE_ON_LAUNCH"], "true");
        assert_eq!(vars["FURNISH_CONFIG__NO_AUTO_UPDATE"], r#"["it/vpn"]"#);
        assert_eq!(vars["FURNISH_CONFIG__FALLBACK"], "ask");
        assert_eq!(vars["FURNISH_CONFIG__COLLECTIONS"], "{}");
        assert_eq!(vars["FURNISH_CONFIG__OS_SPECIFIC__NIXOS__CONFIGURATION"], "/home/alice/cfg");
        assert_eq!(vars["FURNISH_CONFIG__OS_SPECIFIC__NIXOS__FLAKE_ATTRIBUTE"], "desk");
        assert_eq!(vars["FURNISH_CONFIG__OS_SPECIFIC__NIXOS__HOME_MANAGER_CONFIGURATION"], "");
        assert_eq!(vars["FURNISH_DETECTED__OS_SPECIFIC__NIXOS__CONFIGURATION_SOURCE"], "setting");
        assert_eq!(vars["FURNISH_DETECTED__OS_SPECIFIC__NIXOS__HOME_MANAGER_CONFIGURATION"], "");
        assert!(!vars.keys().any(|k| k.starts_with("FURNISH_CONFIG__PARAMETERS")));
    }
}
