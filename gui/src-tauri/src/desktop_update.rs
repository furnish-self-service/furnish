//! A local package manager can replace the app while the old process stays in the tray.
//! Remember its launch location before the first UI request. Probe the installed file as data.
#[cfg(target_os = "linux")]
mod appimage;
use crate::{build_info, tray};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
    sync::{Mutex, OnceLock},
    time::SystemTime,
};

/// No caller-supplied path, command, URL or launch arguments in either desktop command.
static LOCATION: OnceLock<Option<Location>> = OnceLock::new();
static CACHE: Mutex<Option<(Stamp, Option<Installed>)>> = Mutex::new(None);
static RESTART: Mutex<Option<Candidate>> = Mutex::new(None);

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Installed {
    version: String,
    revision: Option<String>,
}

#[derive(Serialize)]
pub struct Status {
    changed: bool,
    installed: Option<Installed>,
}

#[derive(Clone, Debug)]
enum Location {
    Direct(PathBuf),
    #[cfg(target_os = "linux")]
    AppImage(PathBuf),
    #[cfg(unix)]
    Nix {
        launcher: PathBuf,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Candidate {
    binary: PathBuf,
    launcher: PathBuf,
}

pub fn initialize() {
    LOCATION.get_or_init(|| {
        let exe = std::env::current_exe().ok()?;
        if !exe.is_absolute() {
            return None;
        }
        #[cfg(target_os = "linux")]
        let exe = original_linux_path(&exe);
        #[cfg(target_os = "linux")]
        if let Some(image) = appimage::outer_image(Path::new("/proc"), Path::new("/proc/self"), &exe) {
            return Some(Location::AppImage(image));
        }
        #[cfg(unix)]
        if exe.starts_with("/nix/store") {
            return nix_location(&exe, &nix_launchers(), std::env::args_os().next().as_deref().map(Path::new));
        }
        #[cfg(windows)]
        let exe = exe.canonicalize().ok()?;
        Some(Location::Direct(exe))
    });
}

#[cfg(target_os = "linux")]
fn original_linux_path(exe: &Path) -> PathBuf {
    use std::os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::MetadataExt,
    };
    // /proc/self/exe adds a suffix when the old inode was unlinked. A literal filename with
    // that suffix is legal: compare inode numbers before stripping it.
    let same = fs::metadata(exe)
        .ok()
        .zip(fs::metadata("/proc/self/exe").ok())
        .is_some_and(|(a, b)| (a.dev(), a.ino()) == (b.dev(), b.ino()));
    if !same && let Some(original) = exe.as_os_str().as_bytes().strip_suffix(b" (deleted)") {
        return std::ffi::OsString::from_vec(original.to_vec()).into();
    }
    exe.to_owned()
}

#[cfg(unix)]
fn nix_launchers() -> Vec<PathBuf> {
    let mut paths = vec![];
    if let Some(user) = furnish::nixos_apps::current_account() {
        paths.push(user.home.join(".nix-profile/bin/furnish-gui"));
        paths.push(user.home.join(".local/state/nix/profiles/profile/bin/furnish-gui"));
        paths.push(Path::new("/etc/profiles/per-user").join(&user.name).join("bin/furnish-gui"));
        paths.push(Path::new("/nix/var/nix/profiles/per-user").join(user.name).join("profile/bin/furnish-gui"));
    }
    paths.extend([
        "/run/current-system/sw/bin/furnish-gui".into(),
        "/nix/var/nix/profiles/default/bin/furnish-gui".into(),
    ]);
    paths
}

#[cfg(unix)]
fn nix_location(exe: &Path, profiles: &[PathBuf], invoked: Option<&Path>) -> Option<Location> {
    let matches = |alias: &&PathBuf| {
        alias.canonicalize().is_ok_and(|wrapper| {
            exe == wrapper
                || (wrapper.parent() == exe.parent() && exe.file_name().is_some_and(|n| n == ".furnish-gui-wrapped"))
        })
    };
    // argv[0] can only disambiguate aliases independently confirmed by the OS at startup.
    let launcher = profiles
        .iter()
        .filter(matches)
        .find(|alias| Some(alias.as_path()) == invoked)
        .or_else(|| profiles.iter().find(matches))?;
    Some(Location::Nix { launcher: launcher.clone() })
}

impl Location {
    fn candidate(&self) -> Option<Candidate> {
        match self {
            Self::Direct(launcher) => {
                // This path came from the kernel. Never reinterpret a replacement symlink as a
                // request to launch a different program elsewhere.
                if launcher.canonicalize().ok()? != *launcher {
                    return None;
                }
                Some(Candidate { launcher: launcher.clone(), binary: launcher.clone() })
            }
            #[cfg(target_os = "linux")]
            Self::AppImage(path) => Location::Direct(path.clone()).candidate(),
            #[cfg(unix)]
            Self::Nix { launcher } => nix_candidate(launcher, Path::new("/nix/store")),
        }
    }
}

#[cfg(unix)]
fn nix_candidate(launcher: &Path, store: &Path) -> Option<Candidate> {
    let wrapper = launcher.canonicalize().ok()?;
    if !wrapper.starts_with(store) || wrapper.file_name()? != "furnish-gui" {
        return None;
    }
    let inner = wrapper.with_file_name(".furnish-gui-wrapped");
    let binary = if inner.is_file() { inner } else { wrapper.clone() };
    // Pin inspection and launch to one generation; the wrapper sets its own environment.
    Some(Candidate { binary, launcher: wrapper })
}

#[derive(PartialEq, Eq)]
struct Stamp {
    binary: PathBuf,
    len: u64,
    modified: SystemTime,
    #[cfg(unix)]
    unix_id: (u64, u64, i64, i64),
    #[cfg(windows)]
    file_id: (u32, u32, u32),
}

impl Stamp {
    fn of(path: &Path) -> Option<Self> {
        let m = fs::metadata(path).ok()?;
        if !m.is_file() {
            return None;
        }
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        #[cfg(windows)]
        let file_id = {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle};
            let file = fs::File::open(path).ok()?;
            let mut info = BY_HANDLE_FILE_INFORMATION::default();
            // SAFETY: file owns an open handle and info is writable for the call.
            if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
                return None;
            }
            (info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow)
        };
        #[cfg(unix)]
        if m.mode() & 0o111 == 0 {
            return None;
        }
        Some(Self {
            binary: path.to_owned(),
            len: m.len(),
            modified: m.modified().ok()?,
            #[cfg(unix)]
            unix_id: (m.dev(), m.ino(), m.ctime(), m.ctime_nsec()),
            #[cfg(windows)]
            file_id,
        })
    }
}

fn executable(path: &Path) -> io::Result<bool> {
    let mut magic = [0; 4];
    fs::File::open(path)?.read_exact(&mut magic)?;
    #[cfg(target_os = "macos")]
    {
        Ok(matches!(
            u32::from_be_bytes(magic),
            0xfeedfacf | 0xcffaedfe | 0xcafebabe | 0xbebafeca | 0xcafebabf | 0xbfbafeca
        ))
    }
    #[cfg(windows)]
    {
        Ok(magic.starts_with(b"MZ"))
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Ok(&magic == b"\x7fELF")
    }
}

fn changed(running: &Installed, installed: &Installed) -> bool {
    running.version != installed.version
        || running.revision.as_ref().zip(installed.revision.as_ref()).is_some_and(|(a, b)| a != b)
}

fn legacy_build(file: &Path, running: &build_info::BuildInfo) -> Option<Installed> {
    let version = furnish::selfupdate::legacy_desktop_version(file)?;
    // Older MSI/Info.plist metadata can contain only the base version of an alpha build.
    // Within the same base that cannot identify a different executable version.
    let legacy = furnish::selfupdate::semver(&version)?;
    let current = furnish::selfupdate::semver(&running.version)?;
    if legacy.pre.is_empty()
        && !current.pre.is_empty()
        && (legacy.major, legacy.minor, legacy.patch) == (current.major, current.minor, current.patch)
    {
        return None;
    }
    Some(Installed { version, revision: None })
}

fn probe(fresh: bool) -> Option<(Candidate, Installed)> {
    let location = LOCATION.get()?.as_ref()?;
    let candidate = location.candidate()?;
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let installed = replacement(&candidate.binary, build_info::current(), &mut cache, fresh, |binary, _target| {
        #[cfg(target_os = "linux")]
        if matches!(location, Location::AppImage(_)) {
            return Ok(appimage::read_build(binary, Path::new("/usr/bin/unsquashfs"), _target));
        }
        build_info::read_binary(binary)
    })?;
    Some((candidate, installed))
}

fn replacement(
    binary: &Path,
    running: build_info::BuildInfo,
    cache: &mut Option<(Stamp, Option<Installed>)>,
    fresh: bool,
    read: impl FnOnce(&Path, &str) -> io::Result<Option<build_info::BuildInfo>>,
) -> Option<Installed> {
    let stamp = Stamp::of(binary)?;
    if fresh || cache.as_ref().is_none_or(|(last, _)| last != &stamp) {
        if !executable(binary).ok()? {
            return None;
        }
        let marker = read(binary, &running.target).ok()?;
        // A fat binary can have several records. The reader prefers the running target.
        if marker.as_ref().is_some_and(|m| m.target != running.target) {
            return None;
        }
        let embedded = marker.map(|m| Installed { version: m.version, revision: m.revision });
        if Stamp::of(binary).as_ref() != Some(&stamp) {
            return None;
        }
        *cache = Some((stamp, embedded));
    }
    let (_, embedded) = cache.as_ref()?;
    // Package databases may be updated after the executable. Only the marker result is
    // immutable for this file identity.
    let installed = embedded.clone().or_else(|| legacy_build(binary, &running))?;
    changed(&Installed { version: running.version, revision: running.revision }, &installed).then_some(installed)
}

#[tauri::command]
pub async fn desktop_update_status() -> crate::CmdResult<Status> {
    crate::blocking(|| {
        let installed = probe(false).map(|(_, build)| build);
        Ok(Status { changed: installed.is_some(), installed })
    })
    .await
}

#[tauri::command]
pub async fn restart_desktop(app: tauri::AppHandle, expected: Installed) -> crate::CmdResult<bool> {
    crate::blocking(move || {
        let Some(idle) = tray::IdleRestart::reserve(&app) else { return Ok(false) };
        let Some((candidate, installed)) = probe(true) else { return Ok(false) };
        if installed != expected {
            return Ok(false);
        }
        *RESTART.lock().unwrap_or_else(|e| e.into_inner()) = Some(candidate);
        idle.commit();
        app.exit(0);
        Ok(true)
    })
    .await
}

/// run_return has already released Tauri's named mutex, D-Bus name or socket. A healthy
/// replacement can now become the first instance, including when the older app predates this
/// handoff. Don't replay a previous deep link or the sign-in launcher's --hidden argument.
pub fn finish(code: i32) -> ExitCode {
    if let Some(candidate) = RESTART.lock().unwrap_or_else(|e| e.into_inner()).take()
        && let Err(error) = launch(&candidate.launcher)
    {
        eprintln!("furnish: couldn't reopen {}: {error}", candidate.launcher.display());
        return ExitCode::FAILURE;
    }
    ExitCode::from(code as u8)
}

fn launch(path: &Path) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    if let Some(bundle) = furnish::selfupdate::app_bundle(path) {
        let result = Command::new("/usr/bin/open").arg("-n").arg(bundle).output()?;
        return if result.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(String::from_utf8_lossy(&result.stderr)))
        };
    }
    let mut command = Command::new(path);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(path: &Path, info: &build_info::BuildInfo) {
        let magic = if cfg!(windows) {
            b"MZ\0\0"
        } else if cfg!(target_os = "macos") {
            b"\xcf\xfa\xed\xfe"
        } else {
            b"\x7fELF"
        };
        fs::write(
            path,
            [&magic[..], b"\0FURNISH-DESKTOP-BUILD-v1\x1e", &serde_json::to_vec(info).unwrap(), b"\0"].concat(),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn updating_the_file_changes_the_offer_only_for_a_different_build() {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("gui");
        let mut running = build_info::current();
        running.version = "2.0.0".into();
        running.revision = Some("a".repeat(40));
        let mut cache = None;
        for (version, revision, different) in [
            ("2.0.0", Some("a".repeat(40)), false),
            ("3.0.0", None, true),
            ("1.0.0", None, true),
            ("2.0.0", Some("b".repeat(40)), true),
            ("2.0.0", None, false),
        ] {
            let disk = build_info::BuildInfo {
                version: version.into(),
                revision,
                source_date_epoch: Some(123),
                ..running.clone()
            };
            let replacement_file = dir.path().join("replacement");
            image(&replacement_file, &disk);
            fs::rename(&replacement_file, &binary).unwrap();
            let offer = replacement(&binary, running.clone(), &mut cache, false, |p, _| build_info::read_binary(p));
            assert_eq!(offer.is_some(), different);
            if let Some(offer) = offer {
                assert_eq!(offer.version, disk.version);
                assert_eq!(offer.revision, disk.revision);
            }
        }
        fs::remove_file(&binary).unwrap();
        assert!(replacement(&binary, running, &mut cache, true, |p, _| build_info::read_binary(p)).is_none());
    }

    #[test]
    #[cfg(unix)]
    fn follows_the_original_nix_profile_and_restarts_its_new_wrapper() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().canonicalize().unwrap().join("store");
        let old = store.join("a-furnish-gui/bin");
        let new = store.join("b-furnish-gui/bin");
        let alias = dir.path().join("furnish-gui");
        let unrelated = dir.path().join("unrelated");
        for bin in [&old, &new] {
            fs::create_dir_all(bin).unwrap();
            fs::write(bin.join("furnish-gui"), "#!/bin/sh").unwrap();
            fs::write(bin.join(".furnish-gui-wrapped"), "wrapped").unwrap();
        }
        symlink(old.join("furnish-gui"), &alias).unwrap();
        symlink(new.join("furnish-gui"), &unrelated).unwrap();
        let profiles = [unrelated.clone(), alias.clone()];
        let Some(Location::Nix { launcher }) =
            nix_location(&old.join(".furnish-gui-wrapped"), &profiles, Some(&unrelated))
        else {
            panic!("missing Nix launcher")
        };
        assert_eq!(launcher, alias);
        fs::remove_file(&alias).unwrap();
        symlink(new.join("furnish-gui"), &alias).unwrap();
        let target = nix_candidate(&launcher, &store).unwrap();
        assert_eq!(target.binary, new.join(".furnish-gui-wrapped"));
        assert_eq!(target.launcher, new.join("furnish-gui"));
        fs::remove_file(&alias).unwrap();
        symlink(std::env::current_exe().unwrap(), &alias).unwrap();
        assert!(nix_candidate(&launcher, &store).is_none());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn keeps_the_original_filename_after_linux_unlinks_the_process_image() {
        assert_eq!(original_linux_path(Path::new("/usr/bin/furnish-gui (deleted)")), Path::new("/usr/bin/furnish-gui"));
        let exe = std::env::current_exe().unwrap();
        assert_eq!(original_linux_path(&exe), exe);
    }
}
