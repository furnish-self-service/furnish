//! Host facts used by collection expressions and distribution-specific package names.
use std::env;
#[cfg(unix)]
use std::ffi::OsStr;
#[cfg(any(target_os = "linux", windows, test))]
use std::{fs, path::Path};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Distribution {
    pub id: String,
    pub like: Vec<String>,
}

const UNIX_MANAGERS: [(&str, &str); 6] =
    [("pacman", "pacman"), ("apt", "apt-get"), ("dnf", "dnf"), ("zypper", "zypper"), ("brew", "brew"), ("nix", "nix")];

pub(crate) fn supported_manager(name: &str) -> bool {
    native_manager_priority(name).is_some()
}

/// Host facts list per-user managers after the ordinary system managers on all platforms.
pub(crate) fn native_manager_priority(name: &str) -> Option<usize> {
    match name {
        "winget" => Some(0),
        "mise" => Some(UNIX_MANAGERS.len()),
        "0install" => Some(UNIX_MANAGERS.len() + 1),
        _ => UNIX_MANAGERS.iter().position(|(manager, _)| *manager == name),
    }
}

pub(crate) fn valid_distribution_id(word: &str) -> bool {
    !word.is_empty() && word.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
}

#[cfg(any(target_os = "linux", test))]
fn os_release(primary: &Path, vendor: &Path) -> Distribution {
    let contents = fs::read_to_string(primary).or_else(|e| match e.kind() {
        std::io::ErrorKind::NotFound => fs::read_to_string(vendor),
        _ => Err(e),
    });
    let Ok(contents) = contents else { return Distribution::default() };
    let mut distro = Distribution::default();
    // These fields contain only lowercase identifiers. No shell expansion is needed.
    for (key, value) in contents.lines().filter_map(|line| line.trim().split_once('=')) {
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
            .unwrap_or(value);
        match key {
            "ID" => distro.id = if valid_distribution_id(value) { value.to_owned() } else { String::new() },
            "ID_LIKE" => {
                let ids = value.split_ascii_whitespace();
                distro.like = if ids.clone().all(valid_distribution_id) { ids.map(str::to_owned).collect() } else { Vec::new() };
            }
            _ => {}
        }
    }
    distro
}

pub(crate) fn distribution() -> Distribution {
    #[cfg(target_os = "linux")]
    {
        static DISTRO: std::sync::OnceLock<Distribution> = std::sync::OnceLock::new();
        DISTRO.get_or_init(|| os_release(Path::new("/etc/os-release"), Path::new("/usr/lib/os-release"))).clone()
    }
    #[cfg(not(target_os = "linux"))]
    Distribution::default()
}

pub(crate) fn manager_on_platform(id: &str, windows: bool) -> bool {
    per_user_manager(id) || (id == "winget") == windows
}

pub(crate) fn per_user_manager(id: &str) -> bool {
    matches!(id, "mise" | "0install")
}

pub(crate) fn package_managers() -> Vec<&'static str> {
    let mut managers = system_package_managers();
    if crate::resources::mise::executable().is_some() {
        managers.push("mise");
    }
    if !crate::service::in_service() && std::env::var("FURNISH_SERVICE").as_deref() != Ok("1")
        && crate::resources::common::on_command_path("0install").is_some() {
        managers.push("0install");
    }
    managers
}

// Recheck the executables each time: a preceding dependency or step may have installed one.
#[cfg(unix)]
fn system_package_managers() -> Vec<&'static str> {
    // SAFETY: geteuid has no preconditions.
    let root = unsafe { libc::geteuid() } == 0;
    // Match use_brew in the Unix package resource. The root service on macOS only
    // considers standard installations; it validates ownership when it uses brew.
    let brew_paths: &[&str] = if !root {
        &["/opt/homebrew/bin/brew", "/usr/local/bin/brew"]
    } else if cfg!(target_os = "macos") {
        &["/opt/homebrew/bin/brew", "/usr/local/Homebrew/bin/brew"]
    } else {
        &[]
    };
    unix_managers(env::var_os("PATH").as_deref(), !root, brew_paths)
}

#[cfg(unix)]
fn unix_managers(path: Option<&OsStr>, brew_on_path: bool, brew_paths: &[&str]) -> Vec<&'static str> {
    let on_path = |name| which::which_in(name, path, ".").is_ok();
    UNIX_MANAGERS
        .into_iter()
        .filter(|&(manager, program)| match manager {
            "brew" => (brew_on_path && on_path(program)) || brew_paths.iter().any(|p| which::which(p).is_ok()),
            _ => on_path(program),
        })
        .map(|(manager, _)| manager)
        .collect()
}

#[cfg(any(windows, test))]
fn app_installer_winget(windows_apps: &Path, architecture: &str) -> Option<std::path::PathBuf> {
    let suffix = format!("_{architecture}__8wekyb3d8bbwe");
    fs::read_dir(windows_apps)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_ascii_lowercase();
            let version = name.strip_prefix("microsoft.desktopappinstaller_")?.strip_suffix(&suffix)?;
            let mut parts = version.split('.');
            let mut version = [0u16; 4];
            for part in &mut version {
                *part = parts.next()?.parse().ok()?;
            }
            let exe = entry.path().join("winget.exe");
            (parts.next().is_none() && exe.is_file()).then_some((version, exe))
        })
        .max()
        .map(|(_, path)| path)
}

#[cfg(windows)]
pub(crate) fn winget_path() -> Option<std::path::PathBuf> {
    which::which("winget").ok().or_else(|| {
        // A service normally lacks the user's App Installer alias on PATH. Use the
        // native machine's package, including when Furnish runs under emulation.
        use windows_sys::Win32::System::{
            SystemInformation::{IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_ARM64, IMAGE_FILE_MACHINE_I386},
            Threading::{GetCurrentProcess, IsWow64Process2},
        };
        let (mut process, mut native) = (0, 0);
        // SAFETY: use the current process pseudo-handle and two writable outputs.
        if unsafe { IsWow64Process2(GetCurrentProcess(), &mut process, &mut native) } == 0 {
            return None;
        }
        let arch = match native {
            IMAGE_FILE_MACHINE_AMD64 => "x64",
            IMAGE_FILE_MACHINE_ARM64 => "arm64",
            IMAGE_FILE_MACHINE_I386 => "x86",
            _ => return None,
        };
        let program_files =
            env::var_os("ProgramW6432").or_else(|| env::var_os("ProgramFiles")).unwrap_or_else(|| r"C:\Program Files".into());
        app_installer_winget(&Path::new(&program_files).join("WindowsApps"), arch)
    })
}

#[cfg(windows)]
fn system_package_managers() -> Vec<&'static str> {
    if winget_path().is_some() { vec!["winget"] } else { Vec::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arbitrary_distribution_and_ordered_family_come_from_one_file() {
        let dir = tempfile::tempdir().unwrap();
        let primary = dir.path().join("etc-os-release");
        let vendor = dir.path().join("lib-os-release");
        fs::write(&vendor, "ID=fedora\nID_LIKE=fedora\n").unwrap();
        assert_eq!(os_release(&primary, &vendor).id, "fedora");
        fs::write(&primary, "# ID=ignored\nID=replaced\nID=\"custom.2-derivative_x\"\nID_LIKE='rhel fedora'\n").unwrap();
        assert_eq!(
            os_release(&primary, &vendor),
            Distribution { id: "custom.2-derivative_x".into(), like: vec!["rhel".into(), "fedora".into()] }
        );
        fs::write(&primary, "ID=cachyos\n").unwrap();
        assert!(os_release(&primary, &vendor).like.is_empty());
        fs::write(&primary, "ID=$(hostname)\nID_LIKE=\"fedora ${ID}\"\n").unwrap();
        assert_eq!(os_release(&primary, &vendor), Distribution::default());
    }

    #[cfg(unix)]
    #[test]
    fn managers_follow_the_resource_order_and_detect_new_executables() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = Some(dir.path().as_os_str());
        for tool in ["nix", "apt", "pacman", "brew"] {
            let p = dir.path().join(tool);
            fs::write(&p, "").unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::create_dir(dir.path().join("zypper")).unwrap();
        fs::write(dir.path().join("dnf"), "").unwrap();
        assert_eq!(unix_managers(path, true, &[]), ["pacman", "brew", "nix"]);
        fs::rename(dir.path().join("apt"), dir.path().join("apt-get")).unwrap();
        assert_eq!(unix_managers(path, false, &[]), ["pacman", "apt", "nix"]);
        let brew = dir.path().join("brew");
        assert_eq!(unix_managers(None, false, &[brew.to_str().unwrap()]), ["brew"]);
    }

    #[test]
    fn windows_service_finds_newest_app_installer_for_native_architecture() {
        let temp = tempfile::tempdir().unwrap();
        let package = |version: &str, arch: &str| {
            let dir = temp.path().join(format!("Microsoft.DesktopAppInstaller_{version}_{arch}__8wekyb3d8bbwe"));
            fs::create_dir(&dir).unwrap();
            let exe = dir.join("winget.exe");
            fs::write(&exe, "").unwrap();
            exe
        };
        fs::write(temp.path().join("winget.exe"), "").unwrap();
        package("9.9.9.9", "x64");
        let x64 = package("10.0.0.0", "x64");
        let arm = package("1.6.0.1", "arm64");
        package("malformed", "arm64");
        assert_eq!(app_installer_winget(temp.path(), "arm64"), Some(arm));
        assert_eq!(app_installer_winget(temp.path(), "x64"), Some(x64));
        assert_eq!(app_installer_winget(temp.path(), "x86"), None);
    }
}
