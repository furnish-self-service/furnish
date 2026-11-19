//! User-scoped mise global tools. Backend-qualified names are opaque arguments to mise.
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    cmp::Ordering,
    path::PathBuf,
    process::{Command, Stdio},
};

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as platform;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as platform;

pub(crate) fn executable() -> Option<PathBuf> {
    if crate::service::in_service() || std::env::var("FURNISH_SERVICE").as_deref() == Ok("1") {
        return None;
    }
    platform::find_mise()
}

struct Mise(PathBuf);

#[derive(Deserialize)]
struct ToolVersion {
    version: String,
    requested_version: Option<String>,
    installed: bool,
    active: bool,
}

#[derive(Deserialize)]
struct Doctor {
    dirs: Dirs,
    activated: bool,
    shims_on_path: bool,
}

#[derive(Deserialize)]
struct Dirs {
    shims: PathBuf,
    data: PathBuf,
}

impl Mise {
    fn command(&self, args: &[&str], offline: bool) -> Result<Command> {
        // A collection's mise.toml, or one in the directory Furnish started in,
        // must never influence user-global state or trigger hooks.
        let home = dirs::home_dir().context("mise needs a home directory for the signed-in user")?;
        let root = home
            .ancestors()
            .last()
            .filter(|p| p.is_absolute())
            .context("mise needs an absolute home directory")?;
        let mut cmd = Command::new(&self.0);
        cmd.args(args)
            .current_dir(root)
            .env("MISE_YES", "1")
            .env("MISE_NO_COLOR", "1")
            .stdin(Stdio::null());
        // An activated development shell can override the global tool with, for
        // example, MISE_NODE_VERSION. Even "ls --global" hides the real request then.
        for (key, _) in std::env::vars_os() {
            if key
                .to_str()
                .is_some_and(|k| k.starts_with("MISE_") && k.ends_with("_VERSION"))
            {
                cmd.env_remove(key);
            }
        }
        if offline {
            cmd.env("MISE_OFFLINE", "1");
        }
        Ok(cmd)
    }

    fn read(&self, args: &[&str], offline: bool) -> Result<String> {
        let out = self
            .command(args, offline)?
            .output()
            .with_context(|| format!("running mise {}", args[0]))?;
        // Doctor reports a nonzero status for unrelated setup warnings. Its JSON still
        // describes the actual shims path (including a user's shims_dir override).
        ensure!(
            out.status.success() || (args[0] == "doctor" && !out.stdout.is_empty()),
            "mise {} failed ({}): {}",
            args[0],
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
        String::from_utf8(out.stdout).context("mise returned non-UTF-8 output")
    }

    fn change(&self, args: &[&str]) -> Result<()> {
        let status = self.command(args, false)?.stdout(Stdio::from(std::io::stderr())).status()?;
        ensure!(status.success(), "mise {} failed ({status})", args[0]);
        Ok(())
    }

    fn versions(&self, name: &str) -> Result<Vec<ToolVersion>> {
        let mut versions: Vec<ToolVersion> = serde_json::from_str(&self.read(&["ls", "--global", "--json", name], true)?)
            .context("reading global tool versions from mise")?;
        if versions.len() > 1 {
            // ls sorts numerically; current reports the precedence used by the actual shim.
            let current = self.read(&["current", name], true)?;
            let order: Vec<_> = current.split_whitespace().collect();
            versions.sort_by_key(|v| order.iter().position(|&s| s == v.version).unwrap_or(usize::MAX));
        }
        Ok(versions)
    }

    fn doctor(&self) -> Result<Doctor> {
        let d: Doctor = serde_json::from_str(&self.read(&["doctor", "--json"], true)?)
            .context("reading the user's effective shims directory from mise doctor")?;
        ensure!(
            d.dirs.shims.is_absolute() && d.dirs.data.is_absolute(),
            "mise returned a relative data or shims directory"
        );
        Ok(d)
    }

    fn offered(&self, args: &[&str]) -> Vec<String> {
        match self.read(args, false) {
            Ok(out) => out
                .lines()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect(),
            Err(e) => {
                eprintln!("furnish: {e}");
                vec![]
            }
        }
    }
}

fn installed(versions: &[ToolVersion]) -> Option<&str> {
    versions.iter().find(|v| v.active && v.installed).map(|v| v.version.as_str())
}

fn external(name: Option<&str>, dirs: Option<&Dirs>) -> Option<PathBuf> {
    let name = name?;
    let mut paths = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    #[cfg(unix)]
    paths.extend(unix::extra_bin_dirs());
    let roots = dirs
        .into_iter()
        .flat_map(|d| [d.shims.clone(), d.data.join("installs")])
        .filter_map(|p| p.canonicalize().ok())
        .collect::<Vec<_>>();
    paths.retain(|p| p.is_absolute() && p.canonicalize().is_ok_and(|p| !roots.iter().any(|root| p.starts_with(root))));
    let paths = std::env::join_paths(paths).ok()?;
    which::which_in(name, Some(paths), dirs::home_dir()?).ok()
}

fn externally_owned(name: &str, path: PathBuf, op: &str, want: bool, what_if: bool) -> Result<Value> {
    let note = format!(
        "{name} was installed outside furnish ({}): uninstall it the way it was installed",
        path.display()
    );
    let mut state = json!({"name": name, "packageManager": "mise", "_exist": true, "managedBy": "other", "path": path});
    if op == "test" {
        state["_inDesiredState"] = want.into();
    }
    if op == "set" && what_if {
        state["_metadata"] = json!({"whatIf":[note],"furnishChange":false});
    }
    if op == "set" && !what_if && !want {
        bail!("{note}");
    }
    Ok(state)
}

pub(crate) fn run(props: &Value, name: &str, op: &str, what_if: bool) -> Result<Value> {
    ensure!(matches!(op, "get" | "set" | "test"), "unknown mise operation '{op}'");
    let report = props["reportVersions"].as_str().unwrap_or("");
    ensure!(
        matches!(report, "" | "latest" | "all"),
        "reportVersions must be latest or all"
    );
    let target = props["targetVersion"].as_str().filter(|s| !s.is_empty());
    if let Some(v) = target {
        ensure!(
            !v.starts_with('-') && v.bytes().all(|b| b.is_ascii_alphanumeric() || b".+~:_^-".contains(&b)),
            "invalid targetVersion for mise"
        );
        // mise's path: and ref: selectors and version ranges are not exact version pins.
        ensure!(
            !v.contains(':') && !matches!(v, "system" | "latest"),
            "mise targetVersion must be an exact version"
        );
    }
    let command = props["command"].as_str().filter(|s| !s.is_empty());
    if let Some(c) = command {
        ensure!(
            !c.starts_with('-')
                && c.bytes().all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
                && c != "."
                && c != "..",
            "command must be a program name without a directory"
        );
    }
    let want = props["_exist"].as_bool().unwrap_or(true);
    let Some(bin) = executable() else {
        if let Some(path) = external(command, None) {
            return externally_owned(name, path, op, want, what_if);
        }
        return Err(super::common::Missing("mise".into()).into());
    };
    let mise = Mise(bin);
    let versions = mise.versions(name)?;
    let current = installed(&versions);
    // A configured version may be missing from disk. It still needs to be removed on uninstall.
    let configured = versions.iter().any(|v| v.requested_version.is_some());
    let mut state = json!({"name": name, "_exist": current.is_some(), "packageManager": "mise"});
    let doctor = if command.is_some() && !configured || want && op != "get" {
        Some(mise.doctor()?)
    } else {
        None
    };
    if !configured && let Some(path) = doctor.as_ref().and_then(|d| external(command, Some(&d.dirs))) {
        return externally_owned(name, path, op, want, what_if);
    }
    if let Some(version) = current {
        state["version"] = version.into();
    }
    let latest = if !report.is_empty() || (want && op != "get" && current.is_some() && target.is_none()) {
        mise.offered(&["latest", name]).into_iter().next()
    } else {
        None
    };
    if !report.is_empty() {
        state["via"] = "mise".into();
        state["versionScheme"] = "generic".into();
        state["supportsVersionPin"] = true.into();
        if let Some(v) = &latest {
            state["latestVersion"] = v.as_str().into();
        }
        if report == "all" {
            state["availableVersions"] = json!(mise.offered(&["ls-remote", name]));
        }
    }
    if op == "get" {
        return Ok(state);
    }

    let change_tool = if !want {
        configured
    } else if let Some(t) = target {
        current != Some(t)
    } else {
        current.is_none()
            || current
                .zip(latest.as_deref())
                .is_some_and(|(c, l)| crate::required::compare(crate::required::Scheme::Generic, l, c) == Ordering::Greater)
    };
    let shims = if want {
        let d = doctor.as_ref().context("missing mise environment")?;
        (!d.activated && !d.shims_on_path && !platform::shims_on_path(&d.dirs.shims)?).then_some(&d.dirs.shims)
    } else {
        None
    };

    if op == "test" {
        state["_inDesiredState"] = (!change_tool && shims.is_none()).into();
        return Ok(state);
    }
    let mut specs = Vec::new();
    if want {
        specs.push(format!("{name}@{}", target.unwrap_or("latest")));
        // Replacing the default must keep the user's secondary tool versions in order.
        for version in versions
            .iter()
            .skip(1)
            .filter(|v| Some(v.version.as_str()) != target.or(latest.as_deref()))
        {
            if let Some(request) = &version.requested_version {
                specs.push(format!("{name}@{request}"));
            }
        }
    } else {
        // A bare "unuse TOOL" also prunes versions that were never configured globally.
        // Pass each literal configured request to let mise keep unrelated versions and
        // the versions still referenced by its other tracked configs.
        specs.extend(
            versions
                .iter()
                .filter_map(|v| v.requested_version.as_ref())
                .map(|v| format!("{name}@{v}")),
        );
    }
    let mut args = vec![if want { "use" } else { "unuse" }, "--global"];
    args.extend(specs.iter().map(String::as_str));
    if what_if {
        let mut steps = Vec::new();
        if let Some(p) = shims {
            steps.push(format!("Add {} to the user's PATH for new shells", p.display()));
        }
        if change_tool {
            steps.push(format!("Run as your user: mise {}", args.join(" ")));
            state["_exist"] = want.into();
            if let Some(v) = target.or(latest.as_deref()).filter(|_| want) {
                state["version"] = v.into();
            } else {
                state.as_object_mut().unwrap().remove("version");
            }
        }
        if steps.is_empty() {
            steps.push(format!("The global mise tool {name} is already in the requested state"));
        }
        state["_metadata"] =
            json!({"whatIf":steps, "furnishChange":change_tool || shims.is_some(), "furnishNewSession":shims.is_some()});
        return Ok(state);
    }
    if let Some(dir) = shims {
        platform::expose_shims(dir)?;
    }
    if change_tool {
        mise.change(&args)?;
        let after = mise.versions(name)?;
        if want {
            let version = installed(&after).with_context(|| format!("mise did not activate {name} globally"))?;
            if let Some(t) = target {
                ensure!(version == t, "mise selected {version} for {name}, expected {t}");
            }
            if target.is_none()
                && let Some(latest) = latest
            {
                ensure!(
                    crate::required::compare(crate::required::Scheme::Generic, version, &latest) != Ordering::Less,
                    "mise selected {version} for {name}, expected the offered update {latest}"
                );
            }
            state["version"] = version.into();
        } else {
            ensure!(
                !after.iter().any(|v| v.requested_version.is_some()),
                "mise still lists a global request for {name}"
            );
            state.as_object_mut().unwrap().remove("version");
        }
        state["_exist"] = want.into();
    }
    if shims.is_some() {
        state["_metadata"] = json!({"furnishChange":true, "furnishNewSession":true});
    }
    Ok(state)
}
