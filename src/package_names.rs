//! Resolve a package's native name after DSC has evaluated the properties for this operation.
use std::process::Command;

use anyhow::{Context, Result, ensure};
use serde_json::Value;

use crate::platform::{self, Distribution};

fn valid_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('-') && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"@._+-".contains(&b))
}

fn expression(value: &Value) -> bool {
    value.as_str().is_some_and(|s| s.starts_with('[') && !s.starts_with("[[") && s.ends_with(']'))
}

/// Check collection syntax, including selectors for platforms other than the current host.
/// Values that come from DSC expressions are checked against the schema after evaluation.
pub(crate) fn validate(props: &Value) -> Result<()> {
    if let Some(keys) = props.as_object() {
        for key in keys.keys() {
            if let Some(manager) = key.strip_suffix("Name").filter(|s| platform::supported_manager(s)) {
                anyhow::bail!("Furnish/Package property {key} was replaced by namesByManager.{manager}");
            }
        }
    }
    for key in ["namesByDistribution", "namesByManager"] {
        let Some(mapping) = props.get(key) else {
            continue;
        };
        ensure!(props["name"].as_str().is_some_and(|s| !s.is_empty()), "Furnish/Package needs properties.name as a fallback");
        if expression(mapping) {
            continue;
        }
        let entries = mapping.as_object().with_context(|| format!("{key} must be a map of package names"))?;
        for (selector, value) in entries {
            if key == "namesByManager" {
                ensure!(platform::supported_manager(selector), "{key}: unknown package manager '{selector}'");
            } else {
                ensure!(platform::valid_distribution_id(selector), "{key}: invalid os-release identifier '{selector}'");
            }
            ensure!(value.as_str().is_some_and(valid_name) || expression(value), "{key}.{selector} must be a package name");
        }
    }
    Ok(())
}

fn resolve<'a>(props: &'a Value, distribution: &Distribution, manager: Option<&str>) -> Result<&'a str> {
    let base = props["name"].as_str().filter(|s| valid_name(s)).context("a valid package name is required")?;
    let name = std::iter::once(&distribution.id)
        .chain(&distribution.like)
        .find_map(|id| props["namesByDistribution"].get(id))
        .or_else(|| manager.and_then(|m| props["namesByManager"].get(m)));
    match name {
        None => Ok(base),
        Some(v) => v.as_str().filter(|s| valid_name(s)).context("invalid resolved package name"),
    }
}

/// DSC starts the resource in the directory of its manifest, beside the bundled backend.
/// Inherit its stdout and stderr so package manager progress and errors reach DSC as they occur.
pub(crate) fn run(props: &Value, operation: &str, what_if: bool) -> Result<String> {
    ensure!(matches!(operation, "get" | "test" | "set"), "unknown package operation '{operation}'");
    validate(props)?;
    let requested = props.get("packageManager").map(|v| v.as_str().context("packageManager must be a name")).transpose()?;
    #[cfg(unix)]
    let manager = requested.or_else(|| platform::package_managers().first().copied());
    #[cfg(windows)]
    let manager = requested.or(Some("winget"));
    if let Some(m) = manager {
        ensure!(
            platform::supported_manager(m) && (m == "winget") == cfg!(windows),
            "package manager '{m}' is unavailable on this platform"
        );
    }

    #[cfg(unix)]
    let mut backend = Command::new("./furnish-package.dsc.resource.sh");
    #[cfg(windows)]
    let mut backend = {
        let mut cmd = Command::new("powershell.exe");
        cmd.args(["-NoLogo", "-NonInteractive", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "./furnish-package.dsc.resource.ps1"]);
        cmd.env_remove("FURNISH_WINGET");
        if let Some(path) = platform::winget_path() {
            cmd.env("FURNISH_WINGET", path);
        }
        cmd
    };
    backend.env("name", resolve(props, &platform::distribution(), manager)?);
    backend.env("packageManager", manager.unwrap_or(""));
    // Only resource input properties may enter the backend environment.
    for key in ["nixpkgs", "command", "reportVersions", "targetVersion", "_exist"] {
        backend.env_remove(key);
        if let Some(value) = props.get(key) {
            if key == "_exist" {
                backend.env(key, value.as_bool().context("_exist must be a boolean")?.to_string());
            } else {
                backend.env(key, value.as_str().with_context(|| format!("{key} must be a string"))?);
            }
        }
    }
    backend.arg(operation);
    if what_if {
        backend.arg(if cfg!(windows) { "what-if" } else { "--what-if" });
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(backend.exec()).context("could not run the Unix package backend")
    }
    #[cfg(windows)]
    std::process::exit(backend.status().context("could not run the Windows package backend")?.code().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exact_distribution_then_ordered_family_then_manager_then_base() {
        let props = json!({"name":"tool","namesByDistribution":{"custom.2":"own","ubuntu":"first","debian":"second"},
            "namesByManager":{"apt":"apt-tool","winget":"Vendor.Tool"}});
        for (id, like, manager, expected) in [
            ("custom.2", vec!["ubuntu", "debian"], Some("apt"), "own"),
            ("linuxmint", vec!["ubuntu", "debian"], Some("apt"), "first"),
            ("unknown", vec!["debian", "ubuntu"], Some("apt"), "second"),
            ("unknown", vec![], Some("apt"), "apt-tool"),
            ("", vec![], Some("winget"), "Vendor.Tool"),
            ("", vec![], Some("nix"), "tool"),
            ("", vec![], None, "tool"),
        ] {
            let distro = Distribution { id: id.into(), like: like.into_iter().map(str::to_owned).collect() };
            assert_eq!(resolve(&props, &distro, manager).unwrap(), expected);
        }
    }

    #[test]
    fn reports_invalid_selectors_names_and_removed_fields_on_any_host() {
        for (fields, error) in [
            (json!({"namesByDistribution":{"debian/sid":"x"}}), "invalid os-release identifier"),
            (json!({"namesByManager":{"winget":"-bad"}}), "must be a package name"),
            (json!({"namesByManager":{"rpm":"x"}}), "unknown package manager"),
            (json!({"namesByManager":["a"]}), "must be a map"),
            (json!({"aptName":"old"}), "replaced by namesByManager.apt"),
        ] {
            let mut props = fields;
            props["name"] = json!("example");
            assert!(validate(&props).unwrap_err().to_string().contains(error));
        }
        assert!(validate(&json!({"namesByManager":{"apt":"x"}})).is_err());
    }
}
