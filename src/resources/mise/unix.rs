//! User executable discovery and persistent shell PATH on Unix.
use anyhow::{Context, Result, bail, ensure};
use serde_json::json;
use std::{
    env, fs,
    path::{Path, PathBuf},
};

pub(super) fn extra_bin_dirs() -> Vec<PathBuf> {
    dirs::home_dir()
        .map(|p| p.join(".local/bin"))
        .into_iter()
        .chain([PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/local/bin")])
        .collect()
}

pub(super) fn find_mise() -> Option<PathBuf> {
    // mise can install plugins and run hooks. Only invoke it as the actual user.
    if unsafe { libc::geteuid() } == 0 {
        return None;
    }
    let mut dirs = env::var_os("PATH")
        .map(|p| env::split_paths(&p).filter(|p| p.is_absolute()).collect::<Vec<_>>())
        .unwrap_or_default();
    dirs.extend(extra_bin_dirs());
    which::which_in("mise", env::join_paths(dirs).ok(), "/").ok()
}

struct ShellSetup {
    files: Vec<PathBuf>,
    line: String,
}

fn setup(shims: &Path) -> Result<ShellSetup> {
    let home = dirs::home_dir().context("mise needs a user home directory")?;
    let shell_path = env::var_os("SHELL").context("set SHELL or add the mise shims directory to PATH")?;
    let shell = Path::new(&shell_path).file_name().and_then(|s| s.to_str()).unwrap_or("");
    shell_setup(
        shell,
        &home,
        shims,
        env::var_os("ZDOTDIR").map(PathBuf::from),
        env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
    )
}

fn shell_setup(
    shell: &str,
    home: &Path,
    shims: &Path,
    zdotdir: Option<PathBuf>,
    config_home: Option<PathBuf>,
) -> Result<ShellSetup> {
    let dir = shims
        .to_str()
        .context("the mise shims directory must be valid UTF-8 to add it to PATH")?;
    ensure!(
        shims.is_absolute() && !dir.contains(['\n', '\r', ':']),
        "invalid mise shims PATH entry"
    );
    if shell == "fish" {
        let file = config_home
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".config"))
            .join("fish/conf.d/furnish-mise.fish");
        let quoted = format!("'{}'", dir.replace('\\', "\\\\").replace('\'', "\\'"));
        return Ok(ShellSetup {
            files: vec![file],
            line: format!("contains -- {quoted} $PATH; or set -gx PATH {quoted} $PATH"),
        });
    }
    let files = match shell {
        "bash" => {
            let login = [".bash_profile", ".bash_login"]
                .iter()
                .map(|n| home.join(n))
                .find(|p| p.exists())
                .unwrap_or_else(|| home.join(".profile"));
            vec![home.join(".bashrc"), login]
        }
        "zsh" => vec![
            zdotdir
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home.to_path_buf())
                .join(".zshenv"),
        ],
        "sh" | "dash" | "ksh" | "ksh93" => vec![home.join(".profile")],
        _ => bail!("add {} to PATH or enable mise shell activation for {shell}", shims.display()),
    };
    let quoted = format!("'{}'", dir.replace('\'', "'\\''"));
    let line = format!("case \":$PATH:\" in *:{quoted}:*) ;; *) export PATH={quoted}:\"$PATH\" ;; esac");
    Ok(ShellSetup { files, line })
}

pub(super) fn shims_on_path(shims: &Path) -> Result<bool> {
    if env::var_os("PATH").is_some_and(|path| env::split_paths(&path).any(|p| p == shims)) {
        return Ok(true);
    }
    // A GUI can keep an old environment for the entire desktop session. Recognize our
    // persistent setup without running the user's shell or sourcing any startup files.
    let ShellSetup { files, line } = setup(shims)?;
    for file in files {
        let plan = super::super::line(&json!({"path": file, "line": line}))?;
        if plan.apply.is_some() || plan.unverified.is_some() {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn expose_shims(shims: &Path) -> Result<()> {
    let ShellSetup { files, line } = setup(shims)?;
    let mut changes = Vec::new();
    for path in files {
        let plan = super::super::line(&json!({"path": path, "line": line}))?;
        if let Some(why) = plan.unverified {
            bail!("{why}; add {} to PATH", shims.display());
        }
        if let Some(apply) = plan.apply {
            let underlying = super::super::common::resolve(&path)?;
            // Respect externally managed dotfiles even when their parent is writable.
            if fs::metadata(&underlying).is_ok_and(|m| m.permissions().readonly()) {
                bail!(
                    "{} is read-only; add {} to PATH in your shell configuration",
                    path.display(),
                    shims.display()
                );
            }
            changes.push((path, apply));
        }
    }
    for (path, apply) in changes {
        apply().with_context(|| {
            format!(
                "updating {}; add {} to PATH in your shell configuration",
                path.display(),
                shims.display()
            )
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_path_entries_preserve_literal_metacharacters_and_do_not_duplicate() {
        let shims = Path::new("/tmp/with space/'`$HOME*(?)'\\mise");
        let ShellSetup { line, files } = shell_setup("bash", Path::new("/tmp"), shims, None, None).unwrap();
        assert!(files.contains(&PathBuf::from("/tmp/.bashrc")));
        let script = format!("{line}\n{line}\nprintf %s \"$PATH\"");
        let result = std::process::Command::new("/bin/sh")
            .args(["-c", &script])
            .env("PATH", "/usr/bin")
            .output()
            .unwrap();
        assert!(result.status.success());
        assert_eq!(
            String::from_utf8(result.stdout).unwrap(),
            format!("{}:/usr/bin", shims.display())
        );
    }

    #[test]
    fn dotfile_selection_respects_the_active_shell() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        let shims = h.join("shims");
        fs::write(h.join(".bash_profile"), "").unwrap();
        assert_eq!(
            shell_setup("bash", h, &shims, None, None).unwrap().files,
            [h.join(".bashrc"), h.join(".bash_profile")]
        );
        assert_eq!(
            shell_setup("zsh", h, &shims, Some(h.join("dot")), None).unwrap().files,
            [h.join("dot/.zshenv")]
        );
        assert_eq!(
            shell_setup("fish", h, &shims, None, Some(h.join("config"))).unwrap().files,
            [h.join("config/fish/conf.d/furnish-mise.fish")]
        );
        assert!(shell_setup("nu", h, &shims, None, None).is_err());
        assert!(shell_setup("bash", h, Path::new("/bin\nfalse"), None, None).is_err());
    }
}
