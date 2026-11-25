//! Resolve a package's native name after DSC has evaluated the properties for this operation.
use std::{
    io::{Seek, Write},
    process::{Command, Stdio},
};

#[cfg(windows)]
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

use crate::platform::{self, Distribution};
use crate::resources::{
    common::Missing,
    note::{Note, note},
};
use crate::script_managers::Script;

/// Collection package-manager IDs are portable JSON property names and command selectors.
pub(crate) fn valid_manager_id(name: &str) -> bool {
    let alnum = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    name.as_bytes().first().is_some_and(|b| alnum(*b)) && name.bytes().all(|b| alnum(b) || b"._-".contains(&b))
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('-') && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"@._+-".contains(&b))
}

// Backend-qualified tool IDs are only passed to mise. Each component is an identifier,
// including scoped names like npm:@scope/tool. Versions are supplied via targetVersion.
fn mise_name(name: &str) -> bool {
    let part = |s: &str| s.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b));
    match name.split_once(':') {
        Some((backend, tool)) => part(backend) && tool.strip_prefix('@').unwrap_or(tool).split('/').all(part),
        None => part(name),
    }
}

fn valid_for(name: &str, manager: Option<&str>) -> bool {
    match manager {
        Some("mise") => mise_name(name),
        Some("0install") => crate::resources::zero_install::valid_launcher(name),
        _ => valid_name(name),
    }
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
    if let Some(folder) = props.get("flake") {
        ensure!(props.get("nixpkgs").is_none(), "use either flake or nixpkgs");
        crate::catalog::flake::relative(folder.as_str().context("flake must be a folder in the collection")?)?;
    }
    if let Some(id) = props.get("packageManager") {
        ensure!(expression(id) || id.as_str().is_some_and(valid_manager_id), "packageManager must be a valid package manager ID");
    }
    if props["packageManager"] == "0install" || props["namesByManager"].get("0install").is_some() {
        ensure!(props.get("feed").is_some(), "0install requires a feed URL");
    }
    if props["packageManager"] == "0install" && props["namesByManager"].get("0install").is_none()
        && !expression(&props["namesByManager"]) && !expression(&props["name"]) {
        ensure!(props["name"].as_str().is_some_and(crate::resources::zero_install::valid_launcher), "invalid Zero Install launcher name");
    }
    if let Some(feed) = props.get("feed") && !expression(feed) {
        crate::resources::zero_install::validate_feed(feed.as_str().context("feed must be an HTTPS URL")?)?;
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
                ensure!(valid_manager_id(selector), "{key}: invalid package manager ID '{selector}'");
            } else {
                ensure!(platform::valid_distribution_id(selector), "{key}: invalid os-release identifier '{selector}'");
            }
            let manager = if key == "namesByManager" { Some(selector.as_str()) } else { None };
            ensure!(value.as_str().is_some_and(|s| valid_for(s, manager)) || expression(value), "{key}.{selector} must be a package name");
        }
    }
    Ok(())
}

/// The package decision is made after DSC evaluates properties. Both the active resource and
/// target-profile checks use this function, with the appropriate ordered list of native managers.
pub(crate) struct Selection<'a> {
    pub name: &'a str,
    pub manager: Option<String>,
    notes: [Note; 2],
}

impl Selection<'_> {
    pub(crate) fn explanation(&self) -> String {
        self.notes.each_ref().map(Note::english).join(". ")
    }

    fn annotate(&self, mut state: Value) -> Result<String> {
        let metadata = &mut state["_metadata"];
        ensure!(metadata.is_null() || metadata.is_object(), "package preview metadata must be an object");
        metadata["furnishPackageSelection"] = self.notes.iter()
            .map(|n| json!({ "code": n.code, "params": n.params, "english": n.english() }))
            .collect();
        Ok(state.to_string())
    }
}

fn preferred_manager<'a>(props: &'a Value, distribution: &Distribution, native: &[&'a str]) -> Option<(&'a str, &'static str)> {
    if let Some(id) = props["packageManager"].as_str() {
        return Some((id, "package.explicit"));
    }
    let default = native.iter().copied().find(|id| !platform::per_user_manager(id));
    let aliases = &props["namesByManager"];
    if native.iter().any(|id| platform::per_user_manager(id) && aliases.get(*id).is_some())
        && (default.is_none() || distribution_name(props, distribution).is_none())
        && let Some(id) = native.iter().copied().find(|id| aliases.get(*id).is_some())
    {
        return Some((id, "package.mappedManager"));
    }
    default.map(|id| (id, "package.native"))
}

fn distribution_name<'p, 'd>(props: &'p Value, distribution: &'d Distribution) -> Option<(&'p Value, &'static str, &'d str)> {
    let names = &props["namesByDistribution"];
    names.get(&distribution.id).map(|v| (v, "package.distroName", distribution.id.as_str()))
        .or_else(|| distribution.like.iter().find_map(|id| names.get(id).map(|v| (v, "package.familyName", id.as_str()))))
}

/// Route an app with an active Zero Install or mise step through the signed-in user's account.
/// Shared or scripted installations can execute multiple resources as one operation.
#[cfg(test)]
pub(crate) fn requires_user(doc: &Value, values: &serde_json::Map<String, Value>) -> bool {
    requires_user_in(doc, values, &[])
}

pub(crate) fn requires_user_in(doc: &Value, values: &serde_json::Map<String, Value>, profiles: &[std::path::PathBuf]) -> bool {
    let applied = std::cell::LazyCell::new(|| crate::options::apply(doc, values));
    let managers = std::cell::LazyCell::new(platform::package_managers);
    let distribution = std::cell::LazyCell::new(platform::distribution);
    let selects = |step: &Value| {
        if !step["type"].as_str().is_some_and(|kind| kind.eq_ignore_ascii_case(crate::catalog::PACKAGE_TYPE)) {
            return false;
        }
        let props = &step["properties"];
        for profile in profiles {
            match crate::package_receipts::lookup(profile, &step["metadata"], props) {
                Ok(Some(owner)) => return platform::per_user_manager(&owner.manager),
                Ok(None) => {}
                // Keep an unreadable receipt in its own account. The resource then reports
                // the read error instead of selecting a manager from the other profile.
                Err(_) => return *profile == crate::catalog::data_dir() && !crate::service::in_service(),
            }
        }
        let manager = &props["packageManager"];
        if let Some(id) = manager.as_str() {
            return if expression(manager) {
                crate::conditions::value_for(doc, id, values).map_or(true, |v| v.as_str().is_some_and(platform::per_user_manager))
            } else { platform::per_user_manager(id) };
        }
        if !expression(&props["namesByManager"]) && !props["namesByManager"].as_object().is_some_and(|m| m.keys().any(|id| platform::per_user_manager(id))) {
            return false;
        }
        let mut selection = props.clone();
        for key in ["namesByManager", "namesByDistribution"] {
            if expression(&props[key]) {
                let Ok(names) = crate::conditions::value_for(doc, props[key].as_str().unwrap(), values) else { return true };
                selection[key] = names;
            }
        }
        preferred_manager(&selection, &distribution, &managers).is_some_and(|(id, _)| platform::per_user_manager(id))
    };
    fn any(steps: &[Value], selects: &impl Fn(&Value) -> bool, applies: &impl Fn(&Value) -> bool) -> bool {
        steps.iter().any(|step| {
            (selects(step) || step["properties"]["resources"].as_array().is_some_and(|nested| any(nested, selects, applies))) && applies(step)
        })
    }
    let applies = |step: &Value| step["condition"].is_null() || crate::engine::applies_here(step, &applied);
    [doc["resources"].as_array(), crate::engine::removal_steps(doc)].into_iter().flatten().any(|steps| any(steps, &selects, &applies))
}

pub(crate) fn select<'a>(
    props: &'a Value, distribution: &Distribution, native: &[&str], scripts: &[&str], windows: bool,
) -> Result<Selection<'a>> {
    if let Some(manager) = props.get("packageManager") {
        ensure!(manager.is_string(), "packageManager must be a name");
    }
    let (manager, reason) = match preferred_manager(props, distribution, native) {
        Some((id, reason)) if platform::supported_manager(id) => {
            ensure!(platform::manager_on_platform(id, windows), "package manager '{id}' is unavailable on this platform");
            (Some(id), reason)
        }
        Some((id, _)) if scripts.contains(&id) => (Some(id), "package.explicitScript"),
        Some((id, _)) => return Err(Missing(format!("package manager {id}")).into()),
        None => match scripts {
            [id] => (Some(*id), "package.onlyScript"),
            [] if windows => (Some("winget"), "package.windows"),
            [] => (None, "package.noManager"),
            _ => anyhow::bail!(
                "multiple collection package managers are available ({}). Set packageManager to choose one",
                scripts.join(", ")
            ),
        },
    };

    let base = props["name"].as_str().filter(|s| valid_name(s) || (manager == Some("mise") && mise_name(s))).context("a valid package name is required")?;
    let distro = if manager == Some("0install") || (manager, reason) == (Some("mise"), "package.mappedManager") { None } else { distribution_name(props, distribution) };
    let matched = distro.or_else(|| manager.and_then(|id| props["namesByManager"].get(id).map(|v| (v, "package.managerName", id))));
    let (name, name_note) = match matched {
        Some((v, code, selector)) => {
            let name = v.as_str().filter(|s| valid_for(s, manager)).context("invalid resolved package name")?;
            (name, note(code, json!({"name": name, "selector": selector})))
        }
        None => {
            ensure!(valid_for(base, manager), "invalid resolved package name");
            (base, note("package.defaultName", json!({"name": base})))
        }
    };
    let manager_note = note(reason, manager.map_or_else(|| json!({}), |id| json!({"manager": id})));
    Ok(Selection { name, manager: manager.map(str::to_owned), notes: [manager_note, name_note] })
}

#[cfg(windows)]
fn powershell_script(path: &Path) -> Result<Command> {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt};
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;

    let argument = crate::catalog::plain_path(path.to_owned());
    // Definition::path verified the canonical target. Win32 normalization of the
    // PowerShell argument must still select that same file (including on shares).
    if argument != path {
        ensure!(argument.canonicalize()?.as_path() == path, "PowerShell cannot address the validated script {}", path.display());
    }
    let mut directory = vec![0u16; 260];
    loop {
        // SAFETY: the API takes a writable buffer and its capacity in UTF-16 code units.
        let size = unsafe { GetSystemDirectoryW(directory.as_mut_ptr(), directory.len() as u32) } as usize;
        if size == 0 {
            return Err(std::io::Error::last_os_error()).context("locating the Windows system directory");
        }
        if size < directory.len() {
            let executable = PathBuf::from(OsString::from_wide(&directory[..size])).join(r"WindowsPowerShell\v1.0\powershell.exe");
            let mut cmd = Command::new(executable);
            cmd.args(["-NoLogo", "-NonInteractive", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]).arg(argument);
            return Ok(cmd);
        }
        directory.resize(size + 1, 0);
    }
}

fn script_command(id: &str, script: &Script) -> Result<Command> {
    if let Some(interpreter) = &script.interpreter {
        let mut cmd = Command::new(interpreter);
        cmd.arg(&script.path);
        return Ok(cmd);
    }
    #[cfg(unix)]
    {
        ensure!(
            script.path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("sh")),
            "package manager '{id}' needs a .sh script on Unix"
        );
        let mut cmd = Command::new("/bin/sh");
        cmd.arg(&script.path);
        Ok(cmd)
    }
    #[cfg(windows)]
    {
        ensure!(
            script.path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("ps1")),
            "package manager '{id}' needs a .ps1 script on Windows"
        );
        powershell_script(&script.path)
    }
}

fn run_script(id: &str, script: &Script, name: &str, props: &Value, operation: &str) -> Result<Value> {
    let mut cmd = script_command(id, script)?;
    let exist = props.get("_exist").map(|v| v.as_bool().context("_exist must be a boolean")).transpose()?.unwrap_or(true);
    let mut request = serde_json::json!({ "name":name, "packageManager":id, "_exist":exist });
    for key in ["reportVersions", "targetVersion"] {
        if let Some(value) = props.get(key) {
            request[key] = value.as_str().with_context(|| format!("{key} must be a string"))?.into();
        }
    }
    // Stage a single UTF-8 JSON line as stdin. Both streams can then progress independently
    // even if a provider emits a large state before it finishes reading its input.
    let mut input = tempfile::tempfile().context("preparing package manager input")?;
    serde_json::to_writer(&mut input, &request)?;
    input.write_all(b"\n")?;
    input.rewind()?;
    let output = cmd
        .arg(operation)
        .stdin(input)
        .stderr(Stdio::inherit())
        .output()
        .with_context(|| format!("running {operation} with package manager '{id}' ({})", script.path.display()))?;
    if !output.status.success() {
        let reason = output.status.code().map(|code| format!("exit code {code}")).unwrap_or_else(|| output.status.to_string());
        anyhow::bail!("package manager '{id}' {operation} failed ({reason})");
    }
    let state: Value =
        serde_json::from_slice(&output.stdout).with_context(|| format!("package manager '{id}' {operation} must return one JSON state"))?;
    ensure!(
        state["name"].as_str() == Some(name) && state["_exist"].is_boolean(),
        "package manager '{id}' {operation} must return name '{name}' and boolean _exist"
    );
    ensure!(
        operation != "test" || state["_inDesiredState"].is_boolean(),
        "package manager '{id}' test must return boolean _inDesiredState"
    );
    Ok(state)
}

/// DSC starts the resource in the directory of its manifest, beside the bundled backend.
/// Capture the backend's JSON state for previews and install receipts. Its stderr streams
/// package manager progress and diagnostics to DSC during the operation.
pub(crate) fn run(props: &Value, operation: &str, what_if: bool) -> Result<String> {
    ensure!(matches!(operation, "get" | "test" | "set"), "unknown package operation '{operation}'");
    ensure!(!what_if || operation == "set", "only set accepts --what-if");
    let shared = props["_metadata"][crate::nixos_apps::SERVICE_PROFILE] == true;
    let dir = if shared { crate::catalog::system_dir() } else { crate::catalog::data_dir() };
    let owner = crate::package_receipts::lookup(&dir, &props["_metadata"], props)?;
    let mut effective;
    let declared = props;
    let props = if let Some(owner) = &owner {
        effective = props.clone();
        owner.apply(&mut effective);
        &effective
    } else { props };
    validate(props)?;
    let native = if props.get("packageManager").is_none() { platform::package_managers() } else { Vec::new() };
    // Resolving collection scripts checks the installed policy and canonical paths. Do this
    // only when the operation needs a script, as ordinary native packages don't rely on them.
    let distribution = platform::distribution();
    let preferred = preferred_manager(props, &distribution, &native).map(|(id, _)| id);
    let delegated = props["_metadata"][crate::nixos_apps::PACKAGE_FINGERPRINT].is_string();
    ensure!(!delegated || preferred == Some("nix"), "this step does not select the Nix package manager");
    let scripts = if preferred.is_some_and(platform::supported_manager) {
        Default::default()
    } else {
        crate::script_managers::active(props)?
    };
    let ids: Vec<_> = scripts.keys().map(String::as_str).collect();
    let mut selection = select(props, &distribution, &native, &ids, cfg!(windows))?;
    if let Some(receipt) = &owner {
        selection.notes = [
            note("package.recorded", json!({"manager":receipt.manager})),
            note("package.recordedName", json!({"name":receipt.name})),
        ];
    }
    let manager = selection.manager.as_deref();
    if let Some(receipt) = &owner && platform::supported_manager(&receipt.manager)
        && !platform::package_managers().contains(&receipt.manager.as_str()) {
        return Err(Missing(format!("package manager {}", receipt.manager)).into());
    }
    let state = run_selected(props, &selection, &scripts, operation, what_if, delegated)?;
    if operation == "set" && !what_if && owner.is_none() && props["_exist"] != false && state["_exist"] == true
        && !matches!(state["managedBy"].as_str(), Some("other" | "nixos" | "home-manager"))
        && !delegated && let Some(id) = manager {
        crate::package_receipts::remember(declared, id, selection.name)?;
    }
    if what_if { selection.annotate(state) } else { Ok(state.to_string()) }
}

fn run_selected(props: &Value, selection: &Selection, scripts: &std::collections::BTreeMap<String, Script>,
    operation: &str, what_if: bool, delegated: bool) -> Result<Value> {
    let manager = selection.manager.as_deref();
    if let Some((id, script)) = manager.and_then(|id| scripts.get_key_value(id)) {
        let state = run_script(id, script, selection.name, props, if what_if { "what-if" } else { operation })?;
        return Ok(state);
    }

    if let Some(user_manager) = manager.filter(|id| platform::per_user_manager(id)) {
        ensure!(!crate::service::in_service() && std::env::var("FURNISH_SERVICE").as_deref() != Ok("1"),
            "{user_manager} installs tools for the signed-in user and cannot run in the furnish service");
        let state = if user_manager == "0install" {
            crate::resources::zero_install::run(props, selection.name, operation, what_if)?
        } else {
            crate::resources::mise::run(props, selection.name, operation, what_if)?
        };
        return Ok(state);
    }

    if manager == Some("nix") {
        let probe = || {
            let out = native_backend(props, selection.name, manager)?.arg("get").stderr(Stdio::inherit()).output()
                .context("checking for an existing Nix package")?;
            if !out.status.success() {
                std::process::exit(out.status.code().unwrap_or(1));
            }
            serde_json::from_slice(&out.stdout).context("reading the installed Nix package state")
        };
        if let Some(state) = crate::nixos_apps::package(props, selection.name, operation, what_if, probe)? {
            return Ok(state);
        }
    }
    ensure!(!delegated, "the current preset package cannot be installed into the Nix configuration");
    let mut backend = native_backend(props, selection.name, manager)?;
    backend.arg(operation);
    if what_if {
        backend.arg(if cfg!(windows) { "what-if" } else { "--what-if" });
    }
    let out = backend.stderr(Stdio::inherit()).output().context("could not run the package backend")?;
    if !out.status.success() {
        std::process::exit(out.status.code().unwrap_or(1));
    }
    serde_json::from_slice(&out.stdout).context("reading the package backend state")
}

fn native_backend(props: &Value, name: &str, manager: Option<&str>) -> Result<Command> {
    #[cfg(unix)]
    let mut backend = Command::new("./furnish-package.dsc.resource.sh");
    #[cfg(windows)]
    let mut backend = {
        let mut cmd = powershell_script(Path::new("./furnish-package.dsc.resource.ps1"))?;
        cmd.env_remove("FURNISH_WINGET");
        if let Some(path) = platform::winget_path() {
            cmd.env("FURNISH_WINGET", path);
        }
        cmd
    };
    backend.env("FURNISH_NIX_SERVICE_PROFILE", if props["_metadata"][crate::nixos_apps::SERVICE_PROFILE] == true { "1" } else { "" });
    backend.env("name", name);
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
    backend.env_remove("FURNISH_LOCAL_FLAKE");
    if manager == Some("nix") && props["flake"].is_string() {
        backend.env("nixpkgs", crate::catalog::flake::reference(props)?);
        backend.env("FURNISH_LOCAL_FLAKE", "1");
    }
    Ok(backend)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn delegated_nix_change_never_runs_another_backend() {
        for manager in ["apt", "winget", "collection-script"] {
            let props = json!({"packageManager":manager, "name":"tool",
                "_metadata":{(crate::nixos_apps::PACKAGE_FINGERPRINT):"unmatched"}});
            assert!(run(&props, "set", false).unwrap_err().to_string().contains("does not select the Nix package manager"));
        }
    }

    #[test]
    fn exact_distribution_then_ordered_family_then_manager_then_base() {
        let props = json!({"name":"tool","namesByDistribution":{"custom.2":"own","ubuntu":"first","debian":"second"},
            "namesByManager":{"apt":"apt-tool","winget":"Vendor.Tool"}});
        for (id, like, manager, expected, source, selector) in [
            ("custom.2", vec!["ubuntu", "debian"], Some("apt"), "own", "package.distroName", Some("custom.2")),
            ("linuxmint", vec!["ubuntu", "debian"], Some("apt"), "first", "package.familyName", Some("ubuntu")),
            ("unknown", vec!["debian", "ubuntu"], Some("apt"), "second", "package.familyName", Some("debian")),
            ("unknown", vec![], Some("apt"), "apt-tool", "package.managerName", Some("apt")),
            ("", vec![], Some("winget"), "Vendor.Tool", "package.managerName", Some("winget")),
            ("", vec![], Some("nix"), "tool", "package.defaultName", None),
            ("", vec![], None, "tool", "package.defaultName", None),
        ] {
            let distro = Distribution { id: id.into(), like: like.into_iter().map(str::to_owned).collect() };
            let choice = select(&props, &distro, manager.as_slice(), &[], manager == Some("winget")).unwrap();
            assert_eq!(choice.name, expected);
            assert_eq!(choice.notes[1].code, source);
            assert_eq!(choice.notes[1].params.get("selector").and_then(Value::as_str), selector);
        }
    }

    #[test]
    fn preview_explains_the_same_manager_as_the_resource_selects() {
        let props = json!({"name":"tool"});
        let distro = Distribution::default();
        for (manager, native, scripts, windows, selected, code) in [
            (Some("apt"), vec!["dnf"], vec![], false, Some("apt"), "package.explicit"),
            (None, vec!["dnf", "apt"], vec!["a"], false, Some("dnf"), "package.native"),
            (Some("b"), vec!["dnf"], vec!["a", "b"], false, Some("b"), "package.explicitScript"),
            (None, vec![], vec!["a"], false, Some("a"), "package.onlyScript"),
            (None, vec![], vec![], true, Some("winget"), "package.windows"),
            (None, vec![], vec![], false, None, "package.noManager"),
        ] {
            let mut props = props.clone();
            if let Some(id) = manager { props["packageManager"] = id.into(); }
            let choice = select(&props, &distro, &native, &scripts, windows).unwrap();
            assert_eq!(choice.manager.as_deref(), selected);
            let state: Value = serde_json::from_str(&choice.annotate(json!({"_metadata":{"whatIf":["Run: query"]}})).unwrap()).unwrap();
            assert_eq!(state["_metadata"]["whatIf"], json!(["Run: query"]));
            let notes = &state["_metadata"]["furnishPackageSelection"];
            assert_eq!(notes[0]["code"], code);
            assert_eq!(notes[0]["params"]["manager"].as_str(), selected);
            assert_eq!(notes[1]["params"]["name"], "tool");
        }
        assert!(select(&props, &distro, &[], &["a","b"], false).err().unwrap().to_string().contains("Set packageManager"));
        let missing = select(&json!({"name":"tool","packageManager":"b"}), &distro, &[], &["a"], false).err().unwrap();
        assert_eq!(missing.downcast_ref::<Missing>().unwrap().0, "package manager b");
        assert!(select(&json!({"name":"tool","packageManager":"winget"}), &distro, &[], &[], false).is_err());
    }

    #[test]
    fn mise_requires_an_explicit_or_mapped_choice_and_accepts_backend_names_only_for_mise() {
        let distro = Distribution { id: "custom".into(), like: vec!["debian".into()] };
        for id in ["github:akuity/kargo", "pipx:magic-wormhole", "aqua:jqlang/jq", "npm:@scope/tool", "node"] {
            let props = json!({"name":id, "packageManager":"mise"});
            assert_eq!(select(&props, &distro, &[], &[], true).unwrap().name, id);
            assert_eq!(select(&props, &distro, &[], &[], false).unwrap().name, id);
        }
        for name in ["--version", "github:../tool", "github:org//tool", "pipx:pkg@latest", "github:org/tool arg"] {
            assert!(select(&json!({"name":name, "packageManager":"mise"}), &distro, &[], &[], false).is_err());
        }
        let aliases = json!({"name":"tool", "namesByManager":{"mise":"github:akuity/kargo", "apt":"apt-tool"}});
        for (native, manager, name) in [
            (vec!["pacman", "apt", "mise"], "apt", "apt-tool"),
            (vec!["pacman", "mise"], "mise", "github:akuity/kargo"),
            (vec!["pacman"], "pacman", "tool"),
        ] {
            let choice = select(&aliases, &distro, &native, &[], false).unwrap();
            assert_eq!((choice.manager.as_deref(), choice.name), (Some(manager), name));
        }
        let mut props = aliases;
        props["namesByDistribution"] = json!({"debian":"native-tool"});
        let choice = select(&props, &distro, &["pacman", "mise"], &[], false).unwrap();
        assert_eq!((choice.manager.as_deref(), choice.name), (Some("pacman"), "native-tool"));
        assert_eq!(choice.notes[0].code, "package.native");
        let choice = select(&props, &distro, &["mise"], &[], false).unwrap();
        assert_eq!((choice.manager.as_deref(), choice.name), (Some("mise"), "github:akuity/kargo"));
        let props = json!({"name":"generic"});
        assert_eq!(select(&props, &distro, &["mise"], &["org"], false).unwrap().manager.as_deref(), Some("org"));
    }

    #[test]
    fn zero_install_is_an_opt_in_package_fallback() {
        let distro = Distribution::default();
        let mut props = json!({"name":"hello", "namesByManager":{"brew":"native-hello","0install":"hello-cli"},
            "feed":"https://example.org/hello.xml"});
        assert!(validate(&props).is_ok());
        for (native, manager, name) in [
            (vec!["brew", "0install"], "brew", "native-hello"),
            (vec!["winget", "0install"], "0install", "hello-cli"),
            (vec!["winget"], "winget", "hello"),
        ] {
            let choice = select(&props, &distro, &native, &[], native.contains(&"winget")).unwrap();
            assert_eq!((choice.manager.as_deref(), choice.name), (Some(manager), name));
        }
        props.as_object_mut().unwrap().remove("feed");
        assert!(validate(&props).is_err());
        let other = json!({"name":"other"});
        let choice = select(&other, &distro, &["0install"], &[], false).unwrap();
        assert!(choice.manager.is_none());
    }

    #[test]
    fn zero_install_runs_as_the_requesting_user() {
        let mut doc = json!({"resources":[{"type":"Furnish/Package","properties":{"packageManager":"0install","name":"tool","feed":"https://example.org/tool.xml"}}]});
        assert!(requires_user(&doc, &Default::default()));
        doc["resources"][0]["condition"] = json!("[equals(1, 2)]");
        assert!(!requires_user(&doc, &Default::default()));
    }

    #[test]
    fn mise_routes_applicable_steps_including_removal_through_the_user() {
        let step = json!({"type":"Furnish/Package", "properties":{"name":"jq","packageManager":"[parameters('provider')]"}});
        let doc = json!({"parameters":{"provider":{"type":"string","defaultValue":"apt"}},
            "resources":[{"type":"Microsoft.DSC/Group","properties":{"resources":[step]}}]});
        for (manager, expected) in [("apt", false), ("mise", true), ("0install", true)] {
            let values = serde_json::Map::from_iter([("provider".into(), json!(manager))]);
            assert_eq!(requires_user(&doc, &values), expected);
        }
        let mut variable = doc.clone();
        variable["variables"]["provider"] = json!("[parameters('provider')]");
        variable["resources"][0]["properties"]["resources"][0]["properties"]["packageManager"] = json!("[variables('provider')]");
        for (manager, expected) in [("apt", false), ("mise", true), ("0install", true)] {
            let values = serde_json::Map::from_iter([("provider".into(), json!(manager))]);
            assert_eq!(requires_user(&variable, &values), expected);
        }
        let step = json!({"type":"Furnish/Package","properties":{"name":"jq","packageManager":"mise"}});
        let mut doc = json!({"resources":[],"metadata":{"furnish":{"uninstall":[step]}}});
        assert!(requires_user(&doc, &Default::default()));
        doc["metadata"]["furnish"]["uninstall"][0]["condition"] = json!("[equals(1, 2)]");
        assert!(!requires_user(&doc, &Default::default()));
    }

    #[test]
    fn script_receives_only_the_operation_and_resolved_input() {
        let directory = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        let (filename, script) = (
            "package tool.sh",
            r#"set -eu
[ "$#" = 1 ]
dir=$(dirname "$0")
cat > "$dir/input"
printf '%s' "$1" > "$dir/operation"
case "$1" in
  set) touch "$dir/mutated" ;;
  test) printf '{"name":"native-tool","_exist":false,"_inDesiredState":false}'; exit ;;
esac
printf '{"name":"native-tool","_exist":true}'
"#,
        );
        #[cfg(windows)]
        let (filename, script) = (
            "package tool.ps1",
            r#"param([string]$Operation)
if ($args.Count) { exit 23 }
[IO.File]::WriteAllText("$PSScriptRoot/input", [Console]::In.ReadToEnd())
[IO.File]::WriteAllText("$PSScriptRoot/operation", $Operation)
if ($Operation -eq 'set') { [IO.File]::WriteAllText("$PSScriptRoot/mutated", '') }
if ($Operation -eq 'test') { [Console]::WriteLine('{"name":"native-tool","_exist":false,"_inDesiredState":false}'); exit }
[Console]::WriteLine('{"name":"native-tool","_exist":true}')
"#,
        );
        let path = directory.path().join(filename);
        std::fs::write(&path, script).unwrap();
        let script = Script { path: path.canonicalize().unwrap(), interpreter: None };
        let props = json!({"name":"generic","namesByManager":{"org.tool":"native-tool"},
            "targetVersion":"2.0","reportVersions":"all","_exist":false,"command":"ignored",
            "_metadata":{"private":"internal-only"}});
        let expected =
            json!({"name":"native-tool","packageManager":"org.tool","targetVersion":"2.0","reportVersions":"all","_exist":false});
        let name = select(&props, &Distribution::default(), &[], &["org.tool"], false).unwrap().name;
        for operation in ["get", "test", "what-if", "set"] {
            let state = run_script("org.tool", &script, name, &props, operation).unwrap();
            assert_eq!(state["name"], "native-tool");
            let input: Value = serde_json::from_slice(&std::fs::read(directory.path().join("input")).unwrap()).unwrap();
            assert_eq!(input, expected);
            assert_eq!(std::fs::read_to_string(directory.path().join("operation")).unwrap(), operation);
            assert_eq!(directory.path().join("mutated").exists(), operation == "set");
        }
    }

    #[test]
    fn configured_interpreter_receives_the_literal_file_and_operation() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("the provider & helpers.custom");
        let interpreter = std::env::current_exe().unwrap();
        let script = Script { path: file.clone(), interpreter: Some(interpreter.clone()) };
        let mut cmd = script_command("example", &script).unwrap();
        cmd.arg("what-if");
        assert_eq!(cmd.get_program(), interpreter);
        let args: Vec<_> = cmd.get_args().collect();
        assert_eq!(args, [file.as_os_str(), std::ffi::OsStr::new("what-if")]);
        #[cfg(unix)]
        {
            std::fs::write(&file, r#"test "$1" = what-if && printf '{"name":"tool","_exist":true}'"#).unwrap();
            let script = Script { path: file, interpreter: Some("/bin/sh".into()) };
            let json = run_script("example", &script, "tool", &json!({}), "what-if").unwrap();
            assert_eq!(json["_exist"], true);
        }
    }

    #[test]
    fn invalid_output_and_failures_name_the_manager_and_operation() {
        let directory = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        let (filename, script) = (
            "tool.sh",
            r#"case "$1" in
get) echo 'progress on stdout';;
test) echo '{"name":"tool","_exist":true}';;
set) exit 23;;
esac
"#,
        );
        #[cfg(windows)]
        let (filename, script) = (
            "tool.ps1",
            r#"param([string]$Operation)
switch ($Operation) {
    'get' { 'progress on stdout' }
    'test' { '{"name":"tool","_exist":true}' }
    'set' { exit 23 }
}
"#,
        );
        let path = directory.path().join(filename);
        std::fs::write(&path, script).unwrap();
        let script = Script { path, interpreter: None };
        for (operation, expected) in [("get", "one JSON state"), ("test", "boolean _inDesiredState"), ("set", "23")] {
            let error = run_script("org-tool", &script, "tool", &json!({}), operation).unwrap_err().to_string();
            assert!(error.contains(&format!("'org-tool' {operation}")) && error.contains(expected), "{error}");
        }
    }

    #[test]
    fn reports_invalid_selectors_names_and_removed_fields_on_any_host() {
        for (fields, error) in [
            (json!({"namesByDistribution":{"debian/sid":"x"}}), "invalid os-release identifier"),
            (json!({"namesByManager":{"winget":"-bad"}}), "must be a package name"),
            (json!({"namesByManager":{"../tool":"x"}}), "invalid package manager ID"),
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
