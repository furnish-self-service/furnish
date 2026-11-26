//! Only the marked, current-user distribution owns its whole installation directory.
use super::*;
use std::{ffi::CString, fs, os::unix::{ffi::OsStrExt, fs::MetadataExt}, process::Command};

const MARKER: &str = ".furnish-user-install";

pub(super) fn root(exe: &Path) -> Option<&Path> {
    let bin = exe.parent()?;
    let root = bin.parent()?;
    if !matches!(exe.file_name()?.to_str()?, "furnish" | "furnish-gui") || bin.file_name()? != "bin" {
        return None;
    }
    // SAFETY: geteuid takes no arguments.
    let user = unsafe { libc::geteuid() };
    if [root, root.parent()?].iter().any(|p| fs::metadata(p).map_or(true, |m| m.uid() != user || m.mode() & 0o022 != 0)) {
        return None;
    }
    let marker = root.join(MARKER);
    if !fs::symlink_metadata(&marker).ok()?.is_file() || fs::read(marker).ok()? != b"furnish-linux-user-v1\n" {
        return None;
    }
    Some(root)
}

pub(super) fn replace(bytes: &[u8]) -> Result<Installed> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let install = root(&exe).context("the running program is outside a private Linux user installation")?;
    let parent = install.parent().context("missing installation parent")?;
    let lock = fs::File::open(parent)?;
    lock.lock().context("locking the Linux user installation")?;
    let running = fs::metadata("/proc/self/exe")?;
    let disk = fs::metadata(&exe)?;
    if (running.dev(), running.ino()) != (disk.dev(), disk.ino()) || root(&exe) != Some(install) {
        bail!("the installed version changed during the update; restart Furnish");
    }
    exchange(install, parent, bytes)?;
    Ok(Installed::Replaced(install.to_owned()))
}

fn exchange(install: &Path, parent: &Path, bytes: &[u8]) -> Result<()> {
    let staging = tempfile::Builder::new().prefix(".furnish-update-").tempdir_in(parent)?;
    let archive = staging.path().join("update.tar.gz");
    fs::write(&archive, bytes)?;
    // The caller has verified the complete archive with the embedded release key.
    if !Command::new("tar").arg("-xzf").arg(&archive).arg("-C").arg(staging.path()).status()?.success() {
        bail!("unpacking the Linux update failed");
    }
    let new = staging.path().join("furnish");
    if root(&new.join("bin/furnish")) != Some(new.as_path()) ||
        ["bin/furnish", "bin/furnish-gui", "lib/furnish/dsc/dsc"].iter().any(|p| !new.join(p).is_file()) {
        bail!("the update does not contain a complete Linux user installation");
    }
    let path = |p: &Path| CString::new(p.as_os_str().as_bytes());
    let (old, next) = (path(install)?, path(&new)?);
    // SAFETY: both path strings live through the call. Exchange is a single filesystem
    // operation: a failure leaves the running distribution intact. TempDir removes the old tree.
    if unsafe { libc::syscall(libc::SYS_renameat2, libc::AT_FDCWD, old.as_ptr(), libc::AT_FDCWD, next.as_ptr(), libc::RENAME_EXCHANGE) } != 0 {
        return Err(std::io::Error::last_os_error()).context("atomically exchanging the Linux user installation");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn only_marked_private_installations_are_managed() {
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp.path().join("furnish");
        fs::create_dir_all(install.join("bin")).unwrap();
        let cli = install.join("bin/furnish");
        assert_eq!(root(&cli), None);
        fs::write(install.join(MARKER), b"furnish-linux-user-v1\n").unwrap();
        fs::set_permissions(&install, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(root(&cli), Some(install.as_path()));
        assert_eq!(root(&install.join("bin/another")), None);
        let package = Probe { package: Some(Method::Deb), ..Default::default() };
        assert_eq!(classify(&cli, "linux", &package), Method::Deb);
        fs::set_permissions(&install, fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(root(&cli), None);
    }

    #[test]
    fn incomplete_archive_leaves_existing_tree_and_cleans_staging() {
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp.path().join("furnish");
        fs::create_dir(&install).unwrap();
        fs::write(install.join("original"), "working").unwrap();
        let archive = Command::new("tar").args(["-czf", "-", "-C"]).arg(tmp.path()).arg("furnish").output().unwrap();
        assert!(archive.status.success());
        assert!(exchange(&install, tmp.path(), &archive.stdout).is_err());
        assert_eq!(fs::read_to_string(install.join("original")).unwrap(), "working");
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 1);
    }
}
