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
    /// Present: the furnish service installs the preset collections' apps (see `service`).
    pub service: Option<Service>,
    /// The desktop app under the organization's name, logo and colors.
    pub branding: Branding,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
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

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
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
    match read() {
        Ok(policy) => policy.unwrap_or_default(),
        Err(err) => {
            eprintln!("warning: ignoring the admin policy: {err:#}");
            Policy::default()
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
    fn defaults_allow_adding_collections() {
        let p: Policy = serde_json::from_str("{}").unwrap();
        assert!(p.collections.users_can_add);
        assert!(serde_json::from_str::<Policy>(r#"{"setings":{}}"#).is_err()); // typos are caught
    }
}
