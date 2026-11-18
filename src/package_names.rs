//! Resolve a package's native name after DSC has evaluated the properties for this operation.
use std::{
    collections::BTreeMap,
    io::{Seek, Write},
    process::{Command, Stdio},
};

#[cfg(windows)]
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde_json::Value;

use crate::platform::{self, Distribution};
use crate::resources::common::Missing;
use crate::script_managers::Script;

/// Collection package-manager IDs are portable JSON property names and command selectors.
pub(crate) fn valid_manager_id(name: &str) -> bool {
    let alnum = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    name.as_bytes().first().is_some_and(|b| alnum(*b)) && name.bytes().all(|b| alnum(b) || b"._-".contains(&b))
}

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
    if let Some(id) = props.get("packageManager") {
        ensure!(expression(id) || id.as_str().is_some_and(valid_manager_id), "packageManager must be a valid package manager ID");
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

/// Only used when no built-in manager was selected. An explicit ID must be available
/// in this collection, even when another script could handle the same package.
fn select_script<'a>(id: Option<&str>, managers: &'a BTreeMap<String, Script>) -> Result<Option<(&'a str, &'a Script)>> {
    let selected = match id {
        Some(id) => Some(managers.get_key_value(id).ok_or_else(|| Missing(format!("package manager {id}")))?),
        None if managers.len() <= 1 => managers.first_key_value(),
        None => anyhow::bail!(
            "multiple collection package managers are available ({}). Set packageManager to choose one",
            managers.keys().map(String::as_str).collect::<Vec<_>>().join(", ")
        ),
    };
    Ok(selected.map(|(id, script)| (id.as_str(), script)))
}

#[cfg(windows)]
fn powershell_script(path: &Path) -> Result<Command> {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt};
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;

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
            cmd.args(["-NoLogo", "-NonInteractive", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]).arg(path);
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

fn run_script(id: &str, script: &Script, name: &str, props: &Value, operation: &str) -> Result<String> {
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
    Ok(state.to_string())
}

/// DSC starts the resource in the directory of its manifest, beside the bundled backend.
/// Inherit its stdout and stderr so package manager progress and errors reach DSC as they occur.
pub(crate) fn run(props: &Value, operation: &str, what_if: bool) -> Result<String> {
    ensure!(matches!(operation, "get" | "test" | "set"), "unknown package operation '{operation}'");
    ensure!(!what_if || operation == "set", "only set accepts --what-if");
    validate(props)?;
    let requested = props.get("packageManager").map(|v| v.as_str().context("packageManager must be a name")).transpose()?;
    let selected = requested.or_else(|| platform::package_managers().first().copied());
    if !selected.is_some_and(platform::supported_manager) {
        let managers = crate::script_managers::active(props)?;
        if let Some((id, script)) = select_script(selected, &managers)? {
            let name = resolve(props, &platform::distribution(), Some(id))?;
            let operation = if what_if { "what-if" } else { operation };
            return run_script(id, script, name, props, operation);
        }
    }
    // With no backend, the native resource can still recognize an external installation
    // by its command and explain which tool a new installation would require.
    let manager = selected.or(if cfg!(windows) { Some("winget") } else { None });
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
        let mut cmd = powershell_script(Path::new("./furnish-package.dsc.resource.ps1"))?;
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
    use std::path::PathBuf;

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
    fn scripts_are_selected_by_id_or_when_only_one_is_available() {
        let mut managers = BTreeMap::new();
        assert!(select_script(None, &managers).unwrap().is_none());
        managers.insert("a".into(), Script { path: PathBuf::from("a.sh"), interpreter: None });
        assert_eq!(select_script(None, &managers).unwrap().unwrap().0, "a");
        let missing = select_script(Some("unavailable"), &managers).unwrap_err();
        assert_eq!(missing.downcast_ref::<Missing>().unwrap().0, "package manager unavailable");
        managers.insert("b".into(), Script { path: PathBuf::from("b.sh"), interpreter: None });
        assert!(select_script(None, &managers).unwrap_err().to_string().contains("Set packageManager to choose one"));
        assert_eq!(select_script(Some("b"), &managers).unwrap().unwrap().0, "b");
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
        let name = resolve(&props, &Distribution::default(), Some("org.tool")).unwrap();
        for operation in ["get", "test", "what-if", "set"] {
            let state: Value = serde_json::from_str(&run_script("org.tool", &script, name, &props, operation).unwrap()).unwrap();
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
            assert_eq!(serde_json::from_str::<Value>(&json).unwrap()["_exist"], true);
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
