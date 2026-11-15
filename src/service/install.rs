//! `furnish service install|uninstall`: register the service with the OS's service manager,
//! pointing at this furnish binary. Needs administrator rights.

use anyhow::{Context, Result, bail};

#[cfg(unix)]
fn run(program: &str, args: &[&str]) -> Result<()> {
    let status = std::process::Command::new(program).args(args).status().with_context(|| format!("running {program}"))?;
    if !status.success() {
        bail!("{program} {} failed ({status})", args.join(" "));
    }
    Ok(())
}

#[cfg(unix)]
fn require_root() -> Result<()> {
    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } != 0 {
        bail!("installing the service needs administrator rights: sudo furnish service …");
    }
    Ok(())
}

/// This furnish binary, which the service will run as root or SYSTEM: refused if anyone but
/// administrators can change it or a folder it is in.
fn exe() -> Result<std::path::PathBuf> {
    let exe = std::env::current_exe()?.canonicalize()?;
    if let Some(problem) = changeable(&exe)? {
        bail!(
            "{problem}; the service would run it with full rights. Install furnish where only administrators can change it (the furnish packages do), and run `furnish service install` from there"
        );
    }
    // Windows: the plain path for the service manager, not the \\?\ form canonicalize returns.
    #[cfg(windows)]
    if let Some(plain) = exe.to_str().and_then(|s| s.strip_prefix(r"\\?\")).filter(|s| !s.starts_with("UNC")) {
        return Ok(plain.into());
    }
    Ok(exe)
}

/// Unix: the file and every folder above it must belong to root and not be writable by group or
/// others (except the admin group; and folders above its own with the sticky bit, like /tmp,
/// where others can't rename what isn't theirs).
#[cfg(unix)]
fn changeable(exe: &std::path::Path) -> Result<Option<String>> {
    use std::os::unix::fs::MetadataExt;
    for (level, path) in exe.ancestors().enumerate() {
        let meta = std::fs::metadata(path).with_context(|| format!("reading {}", path.display()))?;
        let admin_group = meta.gid() == 0 || (cfg!(target_os = "macos") && meta.gid() == 80);
        let others_write = meta.mode() & 0o002 != 0 || (meta.mode() & 0o020 != 0 && !admin_group);
        let sticky = level >= 2 && meta.mode() & 0o1000 != 0;
        if meta.uid() != 0 {
            return Ok(Some(format!("{} belongs to the user with id {}, not root", path.display(), meta.uid())));
        }
        if others_write && !sticky {
            return Ok(Some(format!("users other than root can write to {} (mode {:o})", path.display(), meta.mode() & 0o7777)));
        }
    }
    Ok(None)
}

/// Windows: only SYSTEM, Administrators and TrustedInstaller may change the file or add files
/// to its folder (furnish runs git from there), or rename what the folders above contain.
#[cfg(windows)]
fn changeable(exe: &std::path::Path) -> Result<Option<String>> {
    use super::windows::{CHANGE, REPLACE, changeable_by};
    for (level, path) in exe.ancestors().enumerate() {
        if let Some(who) = changeable_by(path, if level < 2 { CHANGE } else { REPLACE })? {
            return Ok(Some(format!("{who} can change {}", path.display())));
        }
    }
    Ok(None)
}

#[cfg(not(any(unix, windows)))]
fn changeable(_: &std::path::Path) -> Result<Option<String>> {
    Ok(None)
}

// --- Linux: a systemd unit and the polkit action

#[cfg(target_os = "linux")]
const UNIT: &str = "/etc/systemd/system/furnish.service";
#[cfg(target_os = "linux")]
const POLKIT: &str = "/usr/share/polkit-1/actions/io.github.furnish.policy";

#[cfg(target_os = "linux")]
pub fn install() -> Result<()> {
    require_root()?;
    let unit = include_str!("../../packaging/linux/furnish.service").replace("/usr/bin/furnish", &exe()?.to_string_lossy());
    std::fs::write(UNIT, unit).with_context(|| format!("writing {UNIT}"))?;
    if let Some(dir) = std::path::Path::new(POLKIT).parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(POLKIT, include_str!("../../packaging/linux/io.github.furnish.policy")).with_context(|| format!("writing {POLKIT}"))?;
    run("systemctl", &["daemon-reload"])?;
    run("systemctl", &["enable", "furnish"])?;
    run("systemctl", &["restart", "furnish"])
}

#[cfg(target_os = "linux")]
pub fn uninstall() -> Result<()> {
    require_root()?;
    let _ = run("systemctl", &["disable", "--now", "furnish"]);
    for file in [UNIT, POLKIT] {
        let _ = std::fs::remove_file(file);
    }
    run("systemctl", &["daemon-reload"])
}

// --- macOS: a launchd daemon

#[cfg(target_os = "macos")]
const LABEL: &str = "io.github.furnish-self-service.furnish";

#[cfg(target_os = "macos")]
const LOG: &str = "/var/log/furnish.log";

#[cfg(target_os = "macos")]
fn plist_path() -> String {
    format!("/Library/LaunchDaemons/{LABEL}.plist")
}

#[cfg(target_os = "macos")]
pub fn install() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    require_root()?;
    let plist = include_str!("../../packaging/macos/io.github.furnish-self-service.furnish.plist").replace("/usr/local/bin/furnish", &exe()?.to_string_lossy());
    let path = plist_path();
    // The log has details of failures that users don't see: for administrators only.
    std::fs::OpenOptions::new().create(true).append(true).open(LOG).with_context(|| format!("creating {LOG}"))?;
    std::fs::set_permissions(LOG, std::fs::Permissions::from_mode(0o600))?;
    let _ = run("launchctl", &["bootout", &format!("system/{LABEL}")]); // reinstall: replace the running one
    std::fs::write(&path, plist).with_context(|| format!("writing {path}"))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))?;
    run("launchctl", &["bootstrap", "system", &path])
}

#[cfg(target_os = "macos")]
pub fn uninstall() -> Result<()> {
    require_root()?;
    let _ = run("launchctl", &["bootout", &format!("system/{LABEL}")]);
    let _ = std::fs::remove_file(plist_path());
    Ok(())
}

// --- Windows: a service under SYSTEM

#[cfg(windows)]
pub fn install() -> Result<()> {
    use std::time::Duration;
    use windows_service::{
        service::{
            ServiceAccess, ServiceAction, ServiceActionType, ServiceErrorControl, ServiceFailureActions, ServiceFailureResetPeriod, ServiceInfo,
            ServiceStartType, ServiceType,
        },
        service_manager::{ServiceManager, ServiceManagerAccess},
    };
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE)
        .context("installing the service needs administrator rights (an elevated prompt)")?;
    let info = ServiceInfo {
        name: "furnish".into(),
        display_name: "furnish".into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe()?,
        launch_arguments: vec!["service".into()],
        dependencies: vec![],
        account_name: None, // LocalSystem
        account_password: None,
    };
    let access = ServiceAccess::QUERY_STATUS | ServiceAccess::START | ServiceAccess::STOP | ServiceAccess::CHANGE_CONFIG;
    let service = match manager.open_service("furnish", access) {
        Ok(service) => {
            stop(&service)?;
            service.change_config(&info)?;
            service
        }
        Err(_) => manager.create_service(&info, access)?,
    };
    service.set_description("Installs apps your organization provides")?;
    // Restart when it crashes or stops with an error: while it doesn't run, its pipe is free.
    let restart = |secs| ServiceAction { action_type: ServiceActionType::Restart, delay: Duration::from_secs(secs) };
    service.update_failure_actions(ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(24 * 60 * 60)),
        reboot_msg: None,
        command: None,
        actions: Some(vec![restart(5), restart(10), restart(30)]),
    })?;
    service.set_failure_actions_on_non_crash_failures(true)?;
    service.start::<&str>(&[])?;
    Ok(())
}

#[cfg(windows)]
pub fn uninstall() -> Result<()> {
    use windows_service::{
        service::ServiceAccess,
        service_manager::{ServiceManager, ServiceManagerAccess},
    };
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("removing the service needs administrator rights (an elevated prompt)")?;
    let service = manager.open_service("furnish", ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE)?;
    stop(&service)?;
    service.delete()?;
    Ok(())
}

/// Stop the service if it runs, and wait until it has.
#[cfg(windows)]
fn stop(service: &windows_service::service::Service) -> Result<()> {
    use windows_service::service::ServiceState;
    if service.query_status()?.current_state == ServiceState::Stopped {
        return Ok(());
    }
    let _ = service.stop();
    for _ in 0..100 {
        if service.query_status()?.current_state == ServiceState::Stopped {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    bail!("the furnish service didn't stop")
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn install() -> Result<()> {
    bail!("the furnish service isn't available on this OS")
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn uninstall() -> Result<()> {
    install()
}
