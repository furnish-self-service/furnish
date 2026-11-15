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
pub(super) fn changeable(exe: &std::path::Path) -> Result<Option<String>> {
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
pub(super) fn changeable(exe: &std::path::Path) -> Result<Option<String>> {
    use super::windows::{CHANGE, REPLACE, changeable_by};
    for (level, path) in exe.ancestors().enumerate() {
        if let Some(who) = changeable_by(path, if level < 2 { CHANGE } else { REPLACE })? {
            return Ok(Some(format!("{who} can change {}", path.display())));
        }
    }
    Ok(None)
}

#[cfg(not(any(unix, windows)))]
pub(super) fn changeable(_: &std::path::Path) -> Result<Option<String>> {
    Ok(None)
}

// --- Linux: a systemd unit and the polkit action

#[cfg(target_os = "linux")]
const UNIT: &str = "/etc/systemd/system/furnish.service";
#[cfg(target_os = "linux")]
const POLKIT: &str = "/usr/share/polkit-1/actions/io.github.furnish.policy";

#[cfg(target_os = "linux")]
pub fn install(homebrew_sudo: bool) -> Result<()> {
    if homebrew_sudo {
        bail!("--homebrew-sudo is for macOS");
    }
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
pub fn install(homebrew_sudo: bool) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    require_root()?;
    // Before anything changes: a refusal leaves the computer as it was.
    let brew_owner = if homebrew_sudo { Some(homebrew::owner()?) } else { None };
    let plist = include_str!("../../packaging/macos/io.github.furnish-self-service.furnish.plist").replace("/usr/local/bin/furnish", &exe()?.to_string_lossy());
    let path = plist_path();
    // The log has details of failures that users don't see: for administrators only.
    std::fs::OpenOptions::new().create(true).append(true).open(LOG).with_context(|| format!("creating {LOG}"))?;
    std::fs::set_permissions(LOG, std::fs::Permissions::from_mode(0o600))?;
    let _ = run("launchctl", &["bootout", &format!("system/{LABEL}")]); // reinstall: replace the running one
    std::fs::write(&path, plist).with_context(|| format!("writing {path}"))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))?;
    match brew_owner {
        Some(owner) => homebrew::allow_sudo(&owner)?,
        None => homebrew::remove_rule()?, // installing again without the option takes it back
    }
    run("launchctl", &["bootstrap", "system", &path])
}

#[cfg(target_os = "macos")]
pub fn uninstall() -> Result<()> {
    require_root()?;
    let _ = run("launchctl", &["bootout", &format!("system/{LABEL}")]);
    let _ = std::fs::remove_file(plist_path());
    homebrew::remove_rule()
}

/// macOS: the service runs Homebrew as the account that owns it (Homebrew refuses root), see
/// resources/unix/furnish-homebrew.sh. Casks that run installer packages (`pkg`), and the
/// uninstall steps of many casks, call `sudo` from that account, so `--homebrew-sudo` writes a
/// sudo rule for it.
///
/// The rule allows all commands. A narrower one would protect nothing: what Homebrew runs through
/// sudo is `/usr/sbin/installer -pkg <a package the owner downloaded>` (the package's scripts run
/// as root), `/usr/bin/xargs -0 -- /bin/rm --` with paths from the owner, Homebrew's own
/// `rmdir.sh` from the owner's installation, `launchctl`, `pkgutil --forget`, `kextunload`,
/// `chown`, and casks' own uninstall scripts with `sudo: true`, each with `-E` and variables on
/// the command line (which a rule only allows with SETENV). The installer, xargs, rmdir.sh,
/// launchctl and the scripts each give whoever may run them as root full control, and a list
/// would break casks with scripts. So the rule is only safe when the owner can
/// become root anyway and nobody else can run code as the owner: the owner must be an
/// administrator and the installation must be writable only by the owner, root and admins. The
/// check in furnish-homebrew.sh enforces both here and on every use; when it fails there, the
/// service removes the rule.
#[cfg(target_os = "macos")]
mod homebrew {
    use anyhow::{Context, Result, bail};
    use std::{fs, io::Write, os::unix::fs::OpenOptionsExt, process::Command};

    pub const RULE: &str = "/etc/sudoers.d/furnish-homebrew";
    /// Next to the rule: sudo skips files in sudoers.d whose name contains a dot.
    const STAGED: &str = "/etc/sudoers.d/.furnish-homebrew.new";

    /// The account that owns Homebrew, if the service may use it.
    pub fn owner() -> Result<String> {
        let out = Command::new("/bin/sh")
            .args(["-c", include_str!("../../resources/unix/furnish-homebrew.sh"), "furnish-homebrew-check"])
            .output()
            .context("running /bin/sh")?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        if !out.status.success() {
            bail!(
                "can't let the service use Homebrew: {}. See \"macOS: the service and Homebrew\" in furnish's admin guide",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        let owner = stdout.split_whitespace().next().unwrap_or_default().to_string();
        // Checked by the script too; the name goes into a sudoers file.
        if owner.is_empty() || owner.starts_with('-') || !owner.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c)) {
            bail!("unexpected Homebrew owner '{owner}'");
        }
        Ok(owner)
    }

    pub fn rule_text(owner: &str) -> String {
        format!(
            "# Written by `furnish service install --homebrew-sudo`, removed by `furnish service uninstall`.\n\
             # The furnish service runs Homebrew as {owner}, the administrator who owns it. Casks that run\n\
             # installer packages call sudo, and the service has nobody to type a password.\n\
             {owner} ALL = (root) NOPASSWD: ALL\n"
        )
    }

    /// Writes the rule: staged with its final owner and mode, checked by visudo, then renamed into
    /// place, so sudo never reads a broken or half-written file.
    pub fn allow_sudo(owner: &str) -> Result<()> {
        let sudoers = fs::read_to_string("/etc/sudoers").context("reading /etc/sudoers")?;
        if !sudoers.lines().any(|l| {
            let l = l.trim();
            (l.starts_with("#includedir") || l.starts_with("@includedir")) && l.ends_with("/etc/sudoers.d")
        }) {
            bail!("/etc/sudoers doesn't read /etc/sudoers.d, where the Homebrew rule would go");
        }
        let _ = fs::remove_file(STAGED);
        let mut file = fs::OpenOptions::new().write(true).create_new(true).mode(0o440).open(STAGED).with_context(|| format!("creating {STAGED}"))?;
        file.write_all(rule_text(owner).as_bytes())?;
        drop(file);
        std::os::unix::fs::chown(STAGED, Some(0), Some(0))?; // root:wheel
        let valid = Command::new("/usr/sbin/visudo").args(["-c", "-q", "-f", STAGED]).status().context("running visudo")?;
        if !valid.success() {
            let _ = fs::remove_file(STAGED);
            bail!("visudo rejected the Homebrew rule for {owner}");
        }
        fs::rename(STAGED, RULE).with_context(|| format!("writing {RULE}"))?;
        let all = Command::new("/usr/sbin/visudo").args(["-c", "-q"]).status().context("running visudo")?;
        if !all.success() {
            let _ = fs::remove_file(RULE);
            bail!("sudo's configuration is invalid with the Homebrew rule; removed it again");
        }
        println!("✓ The service runs Homebrew as {owner}, who may now use sudo without a password ({RULE})");
        Ok(())
    }

    pub fn remove_rule() -> Result<()> {
        match fs::remove_file(RULE) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e).with_context(|| format!("removing {RULE}")),
            _ => Ok(()),
        }
    }
}

// --- Windows: a service under SYSTEM

#[cfg(windows)]
pub fn install(homebrew_sudo: bool) -> Result<()> {
    use std::time::Duration;
    if homebrew_sudo {
        bail!("--homebrew-sudo is for macOS");
    }
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
pub fn install(_homebrew_sudo: bool) -> Result<()> {
    bail!("the furnish service isn't available on this OS")
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn uninstall() -> Result<()> {
    install(false)
}
