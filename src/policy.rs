//! Admin policy, read from the operating system's native place for managed settings, which
//! ordinary users can't write:
//!
//! - Linux: `/etc/furnish/policy.json`
//! - macOS: managed preferences for `io.github.furnish-self-service.furnish` (deployed by MDM
//!   configuration profiles): `/Library/Managed Preferences/<user>/<id>.plist`, then
//!   `/Library/Managed Preferences/<id>.plist`
//! - Windows: `HKLM\SOFTWARE\Policies\furnish`, string value `Policy` holding the JSON (Group
//!   Policy / Intune)
//!
//! The structure is the same everywhere (JSON, or the equivalent plist dictionary):
//!
//! ```json
//! { "settings": { "updateIntervalMinutes": { "value": 60, "locked": true } },
//!   "collections": { "preset": [{ "name": "acme-it", "url": "git@…" }], "usersCanAdd": false },
//!   "service": { "allowedGroup": "staff" } }
//! ```
//!
//! FURNISH_POLICY_FILE points to a JSON file instead (for tests).

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;

/// Identifier for macOS managed preferences.
pub const ID: &str = "io.github.furnish-self-service.furnish";

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct Policy {
    /// Setting name (as in settings.json) → value and whether users may change it.
    pub settings: BTreeMap<String, Entry>,
    pub collections: Collections,
    /// Option values (see `options`) by collection or `collection/app`: defaults or locked.
    pub parameters: BTreeMap<String, BTreeMap<String, Entry>>,
    /// Per collection: values for the settings in `settings::PER_COLLECTION`, defaults or locked.
    pub collection_settings: BTreeMap<String, BTreeMap<String, Entry>>,
    /// Present: the furnish service installs the preset collections' apps (see `service`).
    pub service: Option<Service>,
    /// The desktop app under the organization's name, logo and colors.
    pub branding: Branding,
    /// The organization's furnish addresses (see `endpoint`): their collections are preset.
    pub endpoints: Vec<String>,
    /// Apps (`collection/app`) installed on first launch, e.g. a VPN client.
    pub auto_install: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct Branding {
    /// Shown instead of "furnish": window title, sidebar, texts.
    pub name: Option<String>,
    /// SVG, PNG, JPEG or WebP: an absolute path, or `<preset collection>/<path>` inside a
    /// collection the policy presets (so it ships with the collection).
    pub logo: Option<String>,
    /// Brand color (`#rrggbb`); the app derives its accent shades from it.
    pub accent: Option<String>,
    /// Brand color for the dark theme; defaults to a lighter `accent`.
    pub accent_dark: Option<String>,
    /// Any theme color by name (`canvas`, `sidebar`, `ink`, `highlight`, …), per theme.
    pub theme: Theme,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Theme {
    pub light: BTreeMap<String, String>,
    pub dark: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct Service {
    /// Members of this group may install without a password. Others need what polkit allows
    /// (`io.github.furnish.install`, by default an administrator password).
    pub allowed_group: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub value: Value,
    /// Users can't change a locked setting; otherwise the value is only the default.
    #[serde(default)]
    pub locked: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct Collections {
    /// Collections every user gets; users can't remove them.
    pub preset: Vec<Preset>,
    /// Whether users may add their own collections. If not, collections they added are ignored.
    pub users_can_add: bool,
    /// SSH public keys (`ssh-ed25519 AAAA…`): preset collections only use commits signed by one.
    pub trusted_keys: Vec<String>,
}

impl Default for Collections {
    fn default() -> Self {
        Self { preset: Vec::new(), users_can_add: true, trusted_keys: Vec::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    pub name: String,
    pub url: String,
}

/// The policy, or none. An unreadable policy is reported and ignored rather than blocking furnish.
// ponytail: fail open on a broken policy; fail closed (refuse to run) if admins prefer that.
pub fn load() -> Policy {
    with_known_endpoints(load_native(), crate::endpoint::known())
}

/// Only what the admin deployed, without the endpoints' defaults.
pub fn load_native() -> Policy {
    match read() {
        Ok(policy) => policy.unwrap_or_default(),
        Err(err) => {
            eprintln!("warning: ignoring the admin policy: {err:#}");
            Policy::default()
        }
    }
}

/// The policy with what these endpoints (the first added first) say; see `with_endpoints`.
pub fn with_known_endpoints(mut policy: Policy, endpoints: Vec<(String, crate::endpoint::Description)>) -> Policy {
    let endpoints = speaking_for_own(&policy, endpoints, local_remote);
    with_endpoints(&mut policy, &endpoints);
    policy
}

/// The endpoints whose defaults apply, each with only the collections it may speak for. Without
/// `usersCanAdd`, only the endpoints the policy names count. An endpoint the user added doesn't
/// speak for a collection the admin presets (directly or through a named endpoint), nor for one
/// of the user's that comes from elsewhere (`local_url`: a collection's git URL on this computer).
pub fn speaking_for_own(
    policy: &Policy,
    endpoints: Vec<(String, crate::endpoint::Description)>,
    local_url: impl Fn(&str) -> Option<String>,
) -> Vec<(String, crate::endpoint::Description)> {
    let named: Vec<String> = policy.endpoints.iter().filter_map(|e| crate::endpoint::origin(e)).collect();
    let mut admins: Vec<String> = policy.collections.preset.iter().map(|p| p.name.clone()).collect();
    admins.extend(endpoints.iter().filter(|(o, _)| named.contains(o)).flat_map(|(_, d)| d.collections.iter().map(|c| c.name.clone())));
    endpoints
        .into_iter()
        .filter(|(origin, _)| policy.collections.users_can_add || named.contains(origin))
        .map(|(origin, mut d)| {
            if !named.contains(&origin) {
                d.collections.retain(|c| !admins.contains(&c.name) && local_url(&c.name).is_none_or(|url| url == c.url));
            }
            (origin, d)
        })
        .collect()
}

/// The known endpoints whose defaults apply now, as `speaking_for_own` leaves them.
pub fn endpoints_in_effect() -> Vec<(String, crate::endpoint::Description)> {
    speaking_for_own(&load_native(), crate::endpoint::known(), local_remote)
}

/// The git URL of a collection the user added, read from its git config (no git process: the
/// policy is loaded often).
fn local_remote(name: &str) -> Option<String> {
    let config = std::fs::read_to_string(crate::catalog::data_dir().join("collections").join(name).join(".git").join("config")).ok()?;
    let mut in_origin = false;
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            in_origin = line == r#"[remote "origin"]"#;
        } else if in_origin && let Some((key, value)) = line.split_once('=') && key.trim() == "url" {
            return Some(value.trim().to_string());
        }
    }
    None
}

/// Settings an endpoint may set for everything, as a default: when and how furnish updates,
/// reports, and the language. Settings in `settings::PER_COLLECTION` apply to its own
/// collections only; others (such as `parameters` or `noAutoUpdate`) aren't an endpoint's to set.
pub const ENDPOINT_SETTINGS: [&str; 6] = ["updateOnLaunch", "updateIntervalMinutes", "loadAwareUpdates", "reporting", "reportHostname", "language"];

/// Add what the organization's endpoints say (as from `speaking_for_own`, the first added first):
/// the collections of endpoints the policy names become presets. Each endpoint's option values,
/// per-collection settings and apps to install apply to its own collections only, and its
/// settings only from `ENDPOINT_SETTINGS`. Branding comes from the first endpoint the policy
/// names that has some, or without such, only from the first endpoint added. All of it fills in
/// what the policy leaves open, as defaults (never locked).
pub fn with_endpoints(policy: &mut Policy, endpoints: &[(String, crate::endpoint::Description)]) {
    let named: Vec<String> = policy.endpoints.iter().filter_map(|e| crate::endpoint::origin(e)).collect();
    let brander = if endpoints.iter().any(|(o, _)| named.contains(o)) {
        named.iter().find_map(|n| endpoints.iter().find(|(o, d)| o == n && d.branding.is_some()))
    } else {
        endpoints.first()
    };
    if policy.branding == Branding::default()
        && let Some((_, d)) = brander
        && let Some(mut b) = d.branding.clone().and_then(|b| serde_json::from_value::<Branding>(b).ok())
    {
        // A logo only from inside the endpoint's own collections.
        let own_logo = b.logo.as_deref().and_then(|l| l.split_once('/')).is_some_and(|(c, _)| d.collections.iter().any(|o| o.name == c));
        if !own_logo {
            b.logo = None;
        }
        policy.branding = b;
    }
    for (origin, d) in endpoints {
        let own: Vec<&str> = d.collections.iter().map(|c| c.name.as_str()).collect();
        let owns = |target: &str| own.contains(&target.split('/').next().unwrap_or_default());
        if named.contains(origin) {
            for c in &d.collections {
                if !policy.collections.preset.iter().any(|p| p.name == c.name) {
                    policy.collections.preset.push(Preset { name: c.name.clone(), url: c.url.clone() });
                }
            }
        }
        for (key, value) in &d.settings {
            let default = || Entry { value: value.clone(), locked: false };
            if ENDPOINT_SETTINGS.contains(&key.as_str()) {
                policy.settings.entry(key.clone()).or_insert_with(default);
            } else if crate::settings::PER_COLLECTION.contains(&key.as_str()) {
                for c in &own {
                    policy.collection_settings.entry(c.to_string()).or_default().entry(key.clone()).or_insert_with(default);
                }
            }
        }
        for (target, values) in d.parameters.iter().filter(|(t, _)| owns(t)) {
            let entries = policy.parameters.entry(target.clone()).or_default();
            for (name, value) in values {
                entries.entry(name.clone()).or_insert(Entry { value: value.clone(), locked: false });
            }
        }
        for app in d.auto_install.iter().filter(|a| a.contains('/') && owns(a)) {
            if !policy.auto_install.contains(app) {
                policy.auto_install.push(app.clone());
            }
        }
    }
}

fn read() -> Result<Option<Policy>> {
    if let Some(path) = std::env::var_os("FURNISH_POLICY_FILE") {
        return parse_json_file(std::path::Path::new(&path)).map(Some);
    }
    native()
}

fn parse_json_file(path: &std::path::Path) -> Result<Policy> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn native() -> Result<Option<Policy>> {
    let path = std::path::Path::new("/etc/furnish/policy.json");
    if !path.exists() {
        return Ok(None);
    }
    parse_json_file(path).map(Some)
}

#[cfg(target_os = "macos")]
fn native() -> Result<Option<Policy>> {
    let user = std::env::var("USER").unwrap_or_default();
    let base = std::path::Path::new("/Library/Managed Preferences");
    for path in [base.join(&user).join(format!("{ID}.plist")), base.join(format!("{ID}.plist"))] {
        if path.exists() {
            return parse_plist(&path).map(Some);
        }
    }
    Ok(None)
}

/// Managed preferences are plists; the dictionary has the same structure as the JSON.
pub fn parse_plist(path: &std::path::Path) -> Result<Policy> {
    plist::from_file(path).with_context(|| format!("parsing {}", path.display()))
}

#[cfg(windows)]
fn native() -> Result<Option<Policy>> {
    use winreg::{RegKey, enums::HKEY_LOCAL_MACHINE};
    let Ok(key) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(r"SOFTWARE\Policies\furnish") else {
        return Ok(None);
    };
    let Ok(json) = key.get_value::<String, _>("Policy") else {
        return Ok(None);
    };
    serde_json::from_str(&json).context(r"parsing HKLM\SOFTWARE\Policies\furnish\Policy").map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_and_json_mean_the_same() {
        let json: Policy = serde_json::from_str(
            r#"{"settings":{"updateIntervalMinutes":{"value":60,"locked":true}},
                "collections":{"preset":[{"name":"acme","url":"git@example.com:acme.git"}],"usersCanAdd":false}}"#,
        )
        .unwrap();
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
  <key>settings</key><dict>
    <key>updateIntervalMinutes</key><dict><key>value</key><integer>60</integer><key>locked</key><true/></dict>
  </dict>
  <key>collections</key><dict>
    <key>preset</key><array><dict><key>name</key><string>acme</string><key>url</key><string>git@example.com:acme.git</string></dict></array>
    <key>usersCanAdd</key><false/>
  </dict>
</dict></plist>"#;
        let dir = std::env::temp_dir().join(format!("furnish-policy-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("policy.plist");
        std::fs::write(&path, xml).unwrap();
        assert_eq!(parse_plist(&path).unwrap(), json);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn endpoints_fill_in_defaults_but_never_override_or_lock() {
        let mut p: Policy = serde_json::from_str(
            r#"{"endpoints": ["furnish.acme.tld"],
                "settings": {"updateIntervalMinutes": {"value": 60, "locked": true}},
                "branding": {"name": "Admin's"}}"#,
        )
        .unwrap();
        let d: crate::endpoint::Description = serde_json::from_value(serde_json::json!({
            "name": "Acme", "issuer": "https://sso.acme.tld", "clientId": "furnish",
            "collections": [{"name": "acme", "url": "https://furnish.acme.tld/git/acme.git"}],
            "branding": {"name": "Acme Apps"},
            "settings": {"updateIntervalMinutes": 5, "loadAwareUpdates": false},
            "parameters": {"acme": {"role": "developer"}},
            "autoInstall": ["acme/vpn"]
        }))
        .unwrap();
        let other = crate::endpoint::Description { collections: vec![], ..d.clone() };
        with_endpoints(&mut p, &[("https://furnish.acme.tld".into(), d), ("https://elsewhere.tld".into(), other)]);
        // Its collections are preset because the policy names it.
        assert_eq!(p.collections.preset, [Preset { name: "acme".into(), url: "https://furnish.acme.tld/git/acme.git".into() }]);
        // The policy's own values win; the endpoint's only fill gaps, unlocked.
        assert_eq!(p.branding.name.as_deref(), Some("Admin's"));
        assert_eq!(p.settings["updateIntervalMinutes"], Entry { value: 60.into(), locked: true });
        assert_eq!(p.settings["loadAwareUpdates"], Entry { value: false.into(), locked: false });
        assert!(!p.parameters["acme"]["role"].locked);
        assert_eq!(p.auto_install, ["acme/vpn"]);
    }

    fn description(v: serde_json::Value) -> crate::endpoint::Description {
        let mut base = serde_json::json!({"name": "X", "issuer": "https://sso.x.tld", "clientId": "furnish", "collections": []});
        base.as_object_mut().unwrap().extend(v.as_object().unwrap().clone());
        serde_json::from_value(base).unwrap()
    }

    #[test]
    fn endpoints_only_set_things_for_their_own_collections() {
        let acme = description(serde_json::json!({
            "collections": [{"name": "acme", "url": "https://furnish.acme.tld/git/acme.git"}],
            "settings": {"reporting": "all", "fallback": "off", "parameters": {"other": {"x": 1}}, "noAutoUpdate": ["other/app"],
                         "collections": {"other": {"fallback": "automatic"}}},
            "parameters": {"acme": {"role": "dev"}, "acme/vpn": {"region": "eu"}, "other": {"role": "admin"}, "other/app": {"x": 1}},
            "autoInstall": ["acme/vpn", "other/app", "acmeevil/app", "acme"]
        }));
        let mut p = Policy::default();
        with_endpoints(&mut p, &[("https://furnish.acme.tld".into(), acme)]);
        assert_eq!(p.auto_install, ["acme/vpn"]);
        assert_eq!(p.parameters.keys().collect::<Vec<_>>(), ["acme", "acme/vpn"]);
        assert_eq!(p.settings.keys().collect::<Vec<_>>(), ["reporting"]);
        // A per-collection setting only for its own collection.
        assert_eq!(p.collection_settings.keys().collect::<Vec<_>>(), ["acme"]);
        assert_eq!(p.collection_settings["acme"]["fallback"], Entry { value: "off".into(), locked: false });
    }

    #[test]
    fn added_endpoints_dont_speak_for_others_collections() {
        let p: Policy = serde_json::from_str(r#"{"collections": {"preset": [{"name": "it", "url": "git@x:it.git"}]}}"#).unwrap();
        let lookalike = description(serde_json::json!({
            "collections": [{"name": "it", "url": "https://furnish-acme.tld/git/it.git"},
                            {"name": "acme", "url": "https://furnish-acme.tld/git/acme.git"},
                            {"name": "own", "url": "https://furnish-acme.tld/git/own.git"}],
            "parameters": {"it": {"a": 1}, "acme": {"a": 1}, "own": {"a": 1}},
            "autoInstall": ["it/x", "acme/x", "own/x"]
        }));
        // The user's collection "acme" comes from the real address.
        let local = |name: &str| (name == "acme").then(|| "https://furnish.acme.tld/git/acme.git".to_string());
        let mut endpoints = speaking_for_own(&p, vec![("https://furnish-acme.tld".into(), lookalike)], local);
        let mut policy = p.clone();
        with_endpoints(&mut policy, &endpoints);
        assert_eq!(policy.auto_install, ["own/x"]);
        assert_eq!(policy.parameters.keys().collect::<Vec<_>>(), ["own"]);
        // Without usersCanAdd, only addresses the policy names count.
        let mut locked_down = p.clone();
        locked_down.collections.users_can_add = false;
        endpoints = speaking_for_own(&locked_down, endpoints, |_| None);
        assert!(endpoints.is_empty());
    }

    #[test]
    fn branding_comes_from_named_or_the_first_added_endpoint() {
        let first = description(serde_json::json!({
            "collections": [{"name": "a", "url": "https://a.tld/git/a.git"}],
            "branding": {"name": "A", "logo": "b/logo.svg"}}));
        let second = description(serde_json::json!({
            "collections": [{"name": "b", "url": "https://b.tld/git/b.git"}],
            "branding": {"name": "B", "logo": "b/logo.svg"}}));
        let both = [("https://a.tld".to_string(), first.clone()), ("https://b.tld".to_string(), second.clone())];
        let mut p = Policy::default();
        with_endpoints(&mut p, &both);
        // The first added; its logo points into another address's collection, so no logo.
        assert_eq!((p.branding.name.as_deref(), p.branding.logo), (Some("A"), None));
        // An endpoint the policy names wins, with a logo from its own collection.
        let mut p: Policy = serde_json::from_str(r#"{"endpoints": ["b.tld"]}"#).unwrap();
        with_endpoints(&mut p, &both);
        assert_eq!((p.branding.name.as_deref(), p.branding.logo.as_deref()), (Some("B"), Some("b/logo.svg")));
        // The first added without branding: none, even if a later one has some.
        let plain = description(serde_json::json!({"collections": []}));
        let mut p = Policy::default();
        with_endpoints(&mut p, &[("https://a.tld".into(), plain), ("https://b.tld".into(), second)]);
        assert_eq!(p.branding, Branding::default());
    }

    #[test]
    fn defaults_allow_adding_collections() {
        let p: Policy = serde_json::from_str("{}").unwrap();
        assert!(p.collections.users_can_add);
        assert!(serde_json::from_str::<Policy>(r#"{"setings":{}}"#).is_err()); // typos are caught
    }
}
