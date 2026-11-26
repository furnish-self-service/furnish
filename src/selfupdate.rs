//! furnish's own updates. Each build knows how it was installed (`detect`) and asks its channel's
//! feed (`feed_url`, one small JSON file next to the downloads) whether there is a newer version.
//!
//! - Installed without updates of their own (the Windows MSI, the macOS app from the .dmg),
//!   furnish installs new versions itself: the desktop app with Tauri's updater, `furnish
//!   self-update` with `install` below. Both check the download's signature against
//!   `PUBLIC_KEY` and the version the signature names before installing anything.
//! - Installed by a package manager (Homebrew, APT, DNF or Zypper), the package manager
//!   updates furnish; furnish only says that a new version is out and how it arrives.
//! - Nix installations follow their flake input and skip the release feed.
//!
//! The `selfUpdate` setting decides: `auto` (install where furnish can), `notify` (only say so)
//! or `off` (don't even ask the feed). Only the admin policy can lock it; furnish addresses can't
//! set it.
//!
//! The feed is Tauri's updater format (`latest.json`), published by CI per channel:
//!
//! ```json
//! { "version": "0.1.0-alpha.146", "notes": "https://…", "pub_date": "2026-11-16T12:00:00Z",
//!   "platforms": { "windows-x86_64": { "url": "https://…/furnish_0.1.0+alpha.146_x64.msi", "signature": "…" },
//!                  "darwin-aarch64": { "url": "https://…/furnish_0.1.0+alpha.146_aarch64.app.tar.gz", "signature": "…" } } }
//! ```
//!
//! Versions there are SemVer with the alpha build as a pre-release (`0.1.0-alpha.146`), so they
//! compare correctly. Builds of main call themselves `0.1.0+alpha.146` (`--version`, the Linux
//! packages), which is how furnish shows every version (`display`).

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

pub use crate::settings::SelfUpdate;

/// The public half of the key CI signs updates with (Tauri's format: base64 of a minisign
/// public key). Must match `plugins.updater.pubkey` in gui/src-tauri/tauri.conf.json (a test
/// checks that).
pub const PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDk2REM2RTFCMEJDN0M4M0EKUldRNnlNY0xHMjdjbHV1UGlQUk1rZUFBMHFDY1ZqdG1JSEVaWHhEMU5mbitSZklYTHYzSlhHRkYK";

/// Where each channel's feed is.
const FEEDS: &str = "https://furnish-self-service.github.io/furnish";

/// This build's version, as furnish shows it (builds of main: `0.1.0+alpha.<build>`).
pub fn current() -> &'static str {
    option_env!("FURNISH_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}

/// A version as SemVer that compares correctly: `0.1.0+alpha.42` becomes `0.1.0-alpha.42`
/// (build metadata doesn't count in comparisons, a pre-release does).
pub fn semver(version: &str) -> Option<semver::Version> {
    let version = version.trim().trim_start_matches('v');
    let version = match version.split_once("+alpha.") {
        Some((base, build)) => format!("{base}-alpha.{build}"),
        None => version.to_string(),
    };
    semver::Version::parse(&version).ok()
}

/// How furnish shows a version: `0.1.0-alpha.42` as `0.1.0+alpha.42`, like `furnish --version`.
pub fn display(version: &semver::Version) -> String {
    match version.pre.as_str().strip_prefix("alpha.") {
        Some(build) if version.build.is_empty() => format!("{}.{}.{}+alpha.{build}", version.major, version.minor, version.patch),
        _ => version.to_string(),
    }
}

/// Builds of main update to newer builds of main; releases (later) to newer releases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Channel {
    Alpha,
    Stable,
}

impl Channel {
    pub fn of(version: &semver::Version) -> Channel {
        if version.pre.as_str().starts_with("alpha") { Channel::Alpha } else { Channel::Stable }
    }

    pub fn selected(current: &str, method: Method, preference: Option<Self>) -> Self {
        if method.updates_itself() && let Some(channel) = preference { return channel; }
        semver(current).map_or(Self::Stable, |version| Self::of(&version))
    }

    /// An explicit channel switch can move to an older version. Within a channel only advance.
    pub fn offers(self, current: &semver::Version, candidate: &semver::Version) -> bool {
        Self::of(candidate) == self && (Self::of(current) != self || candidate > current)
    }

    pub fn name(self) -> &'static str {
        match self {
            Channel::Alpha => "alpha",
            Channel::Stable => "stable",
        }
    }
}

/// The feed for this build's channel; FURNISH_UPDATE_FEED overrides it (tests).
pub fn feed_url(channel: Channel) -> String {
    if let Some(url) = std::env::var("FURNISH_UPDATE_FEED").ok().filter(|u| !u.is_empty()) {
        return url;
    }
    format!("{FEEDS}/{}/latest.json", channel.name())
}

/// HTTPS, or HTTP on this computer only (a test's feed).
pub fn feed_allowed(url: &str) -> bool {
    if url.starts_with("https://") {
        return true;
    }
    let Some(rest) = url.strip_prefix("http://") else { return false };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = match host.rsplit_once(':') {
        Some((h, port)) if port.chars().all(|c| c.is_ascii_digit()) && !h.is_empty() => h,
        _ => host,
    };
    matches!(host, "127.0.0.1" | "localhost" | "[::1]")
}

// --- How furnish was installed

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Method {
    /// The Windows installer (furnish's .msi).
    Msi,
    /// The macOS app, dragged from the .dmg.
    App,
    Homebrew,
    /// The .deb package (APT).
    Deb,
    /// The .rpm package (DNF, Zypper).
    Rpm,
    Nix,
    /// Built from source, an AppImage, or anything else: furnish only says a new version is out.
    Other,
}

impl Method {
    /// Whether furnish installs new versions itself (nothing else would).
    pub fn updates_itself(self) -> bool {
        matches!(self, Method::Msi | Method::App)
    }

    /// For people: how furnish got here.
    pub fn name(self) -> &'static str {
        match self {
            Method::Msi => "the Windows installer",
            Method::App => "the macOS app download",
            Method::Homebrew => "Homebrew",
            Method::Deb => "the APT package",
            Method::Rpm => "the RPM package",
            Method::Nix => "Nix",
            Method::Other => "an unknown method",
        }
    }

    /// For people: how a new version arrives.
    pub fn how(self) -> &'static str {
        match self {
            Method::Msi | Method::App => "furnish self-update",
            Method::Homebrew => "brew upgrade",
            Method::Deb | Method::Rpm => "with your system updates",
            Method::Nix => "nix flake update",
            Method::Other => "install it the way you installed furnish",
        }
    }
}

/// What `classify` needs to know about the system, gathered by `Probe::gather`.
#[derive(Debug, Default, Clone)]
pub struct Probe {
    /// Windows: where furnish's MSI installed it (the Uninstall registry keys).
    pub msi_dirs: Vec<PathBuf>,
    /// macOS: Homebrew's Caskroom has furnish.
    pub cask: bool,
    /// Linux: the package manager whose package contains the program.
    pub package: Option<Method>,
}

impl Probe {
    /// Look at the system; `root` is `/` except in tests, which build a fake one.
    pub fn gather(exe: &Path, os: &str, root: &Path) -> Probe {
        let at = |p: &str| root.join(p.trim_start_matches('/'));
        match os {
            "windows" => Probe { msi_dirs: msi_dirs(), ..Probe::default() },
            "macos" => Probe {
                cask: ["/opt/homebrew/Caskroom/furnish", "/usr/local/Caskroom/furnish"].iter().any(|p| at(p).is_dir()),
                ..Probe::default()
            },
            "linux" => {
                let listed = |list: &str| std::fs::read_to_string(at(list)).is_ok_and(|l| l.lines().any(|f| Path::new(f) == exe));
                let package = if listed("/var/lib/dpkg/info/furnish.list") {
                    Some(Method::Deb)
                } else if root == Path::new("/") && rpm_owns(exe) {
                    Some(Method::Rpm)
                } else {
                    None
                };
                Probe { package, ..Probe::default() }
            }
            _ => Probe::default(),
        }
    }
}

/// How the program at `exe` (canonical) was installed, on `os` (`std::env::consts::OS`).
pub fn classify(exe: &Path, os: &str, probe: &Probe) -> Method {
    if matches!(os, "linux" | "macos") && exe.starts_with("/nix/store") {
        return Method::Nix;
    }
    match os {
        "windows" => {
            // As text, so tests of Windows paths run anywhere.
            let norm = |p: &str| p.trim_start_matches(r"\\?\").trim_end_matches(['\\', '/']).to_lowercase();
            let exe = exe.to_string_lossy();
            let dir = exe.rsplit_once(['\\', '/']).map_or("", |(d, _)| d);
            if probe.msi_dirs.iter().any(|d| norm(&d.to_string_lossy()) == norm(dir)) { Method::Msi } else { Method::Other }
        }
        "macos" => match app_bundle(exe) {
            Some(_) if probe.cask => Method::Homebrew,
            Some(_) => Method::App,
            None => Method::Other,
        },
        "linux" => probe.package.unwrap_or(Method::Other),
        _ => Method::Other,
    }
}

/// How this furnish was installed.
pub fn detect() -> Method {
    let Ok(exe) = std::env::current_exe().and_then(|p| p.canonicalize()) else { return Method::Other };
    let os = std::env::consts::OS;
    classify(&exe, os, &Probe::gather(&exe, os, Path::new("/")))
}

/// The `.app` folder a macOS program is in (`furnish.app/Contents/MacOS/furnish`).
pub fn app_bundle(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    (macos.file_name()? == "MacOS" && contents.file_name()? == "Contents" && app.extension()? == "app").then(|| app.to_path_buf())
}

#[cfg(windows)]
fn msi_packages() -> Vec<(PathBuf, Option<String>)> {
    use winreg::{RegKey, enums::HKEY_LOCAL_MACHINE};
    let Ok(uninstall) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall") else {
        return Vec::new();
    };
    uninstall
        .enum_keys()
        .flatten()
        .filter_map(|name| uninstall.open_subkey(name).ok())
        .filter(|k| k.get_value::<String, _>("DisplayName").is_ok_and(|n| n == "furnish"))
        .filter(|k| k.get_value::<u32, _>("WindowsInstaller").is_ok_and(|v| v == 1))
        .filter_map(|k| {
            let path: String = k.get_value("InstallLocation").ok()?;
            (!path.is_empty()).then(|| (path.into(), k.get_value("DisplayVersion").ok()))
        })
        .collect()
}

#[cfg(windows)]
fn msi_dirs() -> Vec<PathBuf> {
    msi_packages().into_iter().map(|(dir, _)| dir).collect()
}

#[cfg(not(windows))]
fn msi_dirs() -> Vec<PathBuf> {
    Vec::new()
}

/// Version of an older desktop executable whose package predates the embedded build identity.
/// Only consult the installer which actually owns this file.
pub fn legacy_desktop_version(exe: &Path) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        if exe != Path::new("/usr/bin/furnish-gui") { return None; }
        let query = |program: &str, args: &[&str]| -> Option<String> {
            use std::{io::Read, process::Stdio, time::{Duration, Instant}};
            let mut child = std::process::Command::new(program).args(args)
                .env_clear().env("LANG", "C").env("PATH", "/usr/bin:/bin")
                .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
            let until = Instant::now() + Duration::from_secs(2);
            loop {
                match child.try_wait() {
                    Ok(Some(status)) if status.success() => break,
                    Ok(Some(_)) => return None,
                    Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(10)),
                    _ => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return None;
                    }
                }
            }
            let mut output = Vec::new();
            child.stdout.take()?.take(1025).read_to_end(&mut output).ok()?;
            if output.len() > 1024 { return None; }
            String::from_utf8(output).ok()
        };
        let listed = std::fs::read_to_string("/var/lib/dpkg/info/furnish.list").is_ok_and(|list| list.lines().any(|line| Path::new(line) == exe));
        let raw = if listed {
            let record = query("/usr/bin/dpkg-query", &["-W", "-f=${Status}\t${Version}", "furnish"])?;
            let version = record.trim().strip_prefix("install ok installed\t")?;
            version.rsplit_once('-').filter(|(_, release)| release.bytes().all(|b| b.is_ascii_digit())).map_or(version, |(upstream, _)| upstream).to_owned()
        } else {
            let record = query("/usr/bin/rpm", &["-qf", "--queryformat", "%{NAME}\t%{VERSION}", "/usr/bin/furnish-gui"])?;
            record.trim().strip_prefix("furnish\t")?.to_owned()
        };
        semver(&raw.replace("~alpha.", "-alpha.")).map(|v| v.to_string())
    }
    #[cfg(target_os = "macos")]
    {
        let bundle = app_bundle(exe)?;
        let file = std::fs::File::open(bundle.join("Contents/Info.plist")).ok()?;
        if file.metadata().ok()?.len() > 1024 * 1024 { return None; }
        let info = plist::Value::from_reader(std::io::BufReader::new(file)).ok()?;
        let info = info.as_dictionary()?;
        if info.get("CFBundleIdentifier")?.as_string()? != "io.github.furnish-self-service.furnish"
            || info.get("CFBundleExecutable")?.as_string()? != exe.file_name()?.to_str()? { return None; }
        semver(info.get("CFBundleShortVersionString")?.as_string()?).map(|v| v.to_string())
    }
    #[cfg(windows)]
    {
        if !exe.file_name()?.eq_ignore_ascii_case("furnish-gui.exe") { return None; }
        let parent = exe.parent()?.canonicalize().ok()?;
        msi_packages().into_iter().find_map(|(dir, version)| {
            (dir.canonicalize().ok()? == parent).then(|| semver(&version?).map(|v| v.to_string()))?
        })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    { let _ = exe; None }
}

fn rpm_owns(exe: &Path) -> bool {
    std::process::Command::new("rpm")
        .args(["-qf", "--queryformat", "%{NAME}"])
        .arg(exe)
        .stderr(std::process::Stdio::null())
        .output()
        .is_ok_and(|o| o.status.success() && o.stdout == b"furnish")
}

/// What the setting means for this installation: `auto` only installs where furnish can.
pub fn effective(mode: SelfUpdate, method: Method) -> SelfUpdate {
    if method == Method::Nix {
        return SelfUpdate::Off;
    }
    match mode {
        SelfUpdate::Auto if !method.updates_itself() => SelfUpdate::Notify,
        other => other,
    }
}

// --- The feed

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pub_date: Option<String>,
    #[serde(default)]
    pub platforms: BTreeMap<String, Platform>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Platform {
    pub url: String,
    pub signature: String,
}

/// A newer version than this one.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Available {
    /// As furnish shows versions (`display`).
    pub version: String,
    /// A link to what changed.
    pub notes: Option<String>,
}

impl Manifest {
    /// The feed's version if it is newer than `current`.
    pub fn available(&self, current: &str, channel: Channel) -> Option<Available> {
        let theirs = semver(&self.version)?;
        let ours = semver(current)?;
        channel.offers(&ours, &theirs).then(|| Available { version: display(&theirs), notes: self.notes.clone() })
    }
}

/// Read the feed (one small GET).
pub fn fetch(url: &str) -> Result<Manifest> {
    if !feed_allowed(url) {
        bail!("the update feed {url} doesn't use HTTPS");
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build())
        .timeout_global(Some(std::time::Duration::from_secs(20)))
        .build()
        .into();
    let mut response = agent.get(url).call().with_context(|| format!("reading {url}"))?;
    response.body_mut().read_json().with_context(|| format!("reading {url}"))
}

/// A channel without its first release has no manifest. Other endpoints and HTTP errors
/// remain visible, including a missing alpha feed or a misconfigured custom feed.
pub fn published(channel: Channel) -> Result<Option<Manifest>> {
    let url = feed_url(channel);
    published_result(&url, fetch(&url))
}

fn published_result(url: &str, result: Result<Manifest>) -> Result<Option<Manifest>> {
    match result {
        Err(err) if url == format!("{FEEDS}/stable/latest.json")
            && matches!(err.downcast_ref::<ureq::Error>(), Some(ureq::Error::StatusCode(404))) => Ok(None),
        result => result.map(Some),
    }
}

/// Whether the feed for `current` has a newer version.
pub fn check(current: &str, channel: Channel) -> Result<Option<Available>> {
    Ok(published(channel)?.and_then(|m| m.available(current, channel)))
}

/// The feed's key for this system, as Tauri's updater names it.
pub fn target() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        os => os,
    };
    format!("{os}-{}", std::env::consts::ARCH)
}

/// Check a download against its signature (base64, as in the feed) and the public key, and that
/// the signature was made for `version` (its trusted comment names it, `version:<v>`). A feed
/// is not signed itself; without this, it could offer an older signed download as a newer one.
pub fn verify(bytes: &[u8], signature: &str, public_key: &str, version: &str) -> Result<()> {
    use base64::Engine as _;
    let text = |b64: &str, what: &str| -> Result<String> {
        let raw = base64::engine::general_purpose::STANDARD.decode(b64.trim()).with_context(|| format!("the {what} isn't base64"))?;
        String::from_utf8(raw).with_context(|| format!("the {what} isn't text"))
    };
    let key = minisign_verify::PublicKey::decode(&text(public_key, "public key")?).context("reading the public key")?;
    let signature = minisign_verify::Signature::decode(&text(signature, "signature")?).context("reading the signature")?;
    key.verify(bytes, &signature, false).context("the download doesn't match its signature")?;
    let signed = signature.trusted_comment().split('\t').find_map(|f| f.strip_prefix("version:")).context("the signature doesn't name a version")?;
    if semver(signed) != semver(version) || semver(signed).is_none() {
        bail!("the signature is for version {signed}, not {version}");
    }
    Ok(())
}

/// What `install` did.
pub enum Installed {
    /// Windows: the installer runs on its own now (it may ask for administrator rights); this
    /// process should end so the installer can replace it.
    InstallerStarted,
    /// macOS: the new app is in place; it runs from the next start.
    Replaced(PathBuf),
}

/// Download this system's file from the feed, check it and install it (`furnish self-update`).
/// The desktop app uses Tauri's updater instead, with the same feed and key.
pub fn install(manifest: &Manifest, method: Method) -> Result<Installed> {
    if !method.updates_itself() {
        bail!("furnish was installed with {}, which also updates it: {}", method.name(), method.how());
    }
    let target = target();
    let platform = manifest.platforms.get(&target).with_context(|| format!("the update feed has no download for {target}"))?;
    if !feed_allowed(&platform.url) {
        bail!("the download {} doesn't use HTTPS", platform.url);
    }
    let mut response = crate::endpoint::http().get(&platform.url).call().with_context(|| format!("downloading {}", platform.url))?;
    let bytes = response.body_mut().with_config().limit(1 << 30).read_to_vec().with_context(|| format!("downloading {}", platform.url))?;
    verify(&bytes, &platform.signature, PUBLIC_KEY, &manifest.version)?;
    match method {
        Method::Msi => install_msi(&bytes, &manifest.version),
        _ => install_app(&bytes),
    }
}

fn install_msi(bytes: &[u8], version: &str) -> Result<Installed> {
    let path = std::env::temp_dir().join(format!("furnish-{version}.msi"));
    std::fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))?;
    let msiexec = std::env::var_os("SystemRoot").map_or(PathBuf::from("msiexec.exe"), |r| Path::new(&r).join("System32").join("msiexec.exe"));
    // Detached: the installer replaces this program, so it must not wait for it.
    std::process::Command::new(msiexec)
        .arg("/i")
        .arg(&path)
        .args(["/passive", "/norestart"])
        .spawn()
        .context("starting the Windows installer")?;
    Ok(Installed::InstallerStarted)
}

/// Unpack the `.app.tar.gz` beside the running app and swap the two folders.
fn install_app(bytes: &[u8]) -> Result<Installed> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let app = app_bundle(&exe).context("furnish doesn't run from an app folder")?;
    let parent = app.parent().context("the app has no parent folder")?;
    let staging = parent.join(format!(".furnish-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir(&staging).map_err(|e| denied(e, parent))?;
    let result = (|| {
        let archive = staging.join("update.tar.gz");
        std::fs::write(&archive, bytes)?;
        let status = std::process::Command::new("/usr/bin/tar").arg("-xzf").arg(&archive).arg("-C").arg(&staging).status()?;
        if !status.success() {
            bail!("unpacking the update failed ({status})");
        }
        let new = std::fs::read_dir(&staging)?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "app"))
            .context("the update contains no app")?;
        let old = staging.join("previous.app");
        std::fs::rename(&app, &old).map_err(|e| denied(e, &app))?;
        if let Err(err) = std::fs::rename(&new, &app) {
            let _ = std::fs::rename(&old, &app);
            return Err(err.into());
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&staging);
    result.map(|()| Installed::Replaced(app))
}

fn denied(err: std::io::Error, path: &Path) -> anyhow::Error {
    if err.kind() == std::io::ErrorKind::PermissionDenied {
        anyhow::anyhow!("can't replace {}: run `sudo furnish self-update`, or ask your administrator", path.display())
    } else {
        anyhow::Error::new(err).context(format!("replacing {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpha_builds_compare_by_build_number() {
        let v = |s| semver(s).unwrap();
        assert!(v("0.1.0-alpha.146") > v("0.1.0+alpha.145"));
        assert!(v("0.1.0+alpha.146") > v("0.1.0+alpha.145"));
        assert!(v("0.1.0-alpha.1000") > v("0.1.0-alpha.999"));
        assert_eq!(v("0.1.0+alpha.145"), v("v0.1.0-alpha.145"));
        // A release comes after its alphas; a branch build (no alpha) never takes an alpha.
        assert!(v("0.1.0") > v("0.1.0-alpha.999"));
        assert!(v("0.1.1-alpha.1") > v("0.1.0"));
        assert!(semver("not a version").is_none());
        assert_eq!(display(&v("0.1.0-alpha.146")), "0.1.0+alpha.146");
        assert_eq!(display(&v("0.2.0")), "0.2.0");
        assert_eq!(Channel::of(&v("0.1.0+alpha.3")), Channel::Alpha);
        assert_eq!(Channel::of(&v("0.1.0")), Channel::Stable);
    }

    #[test]
    fn unpublished_stable_feed_is_quiet_and_other_failures_are_reported() {
        let missing = || Err(anyhow::Error::new(ureq::Error::StatusCode(404)).context("reading the release manifest"));
        assert!(published_result(&format!("{FEEDS}/stable/latest.json"), missing()).unwrap().is_none());
        assert!(published_result(&format!("{FEEDS}/alpha/latest.json"), missing()).is_err());
        assert!(published_result("https://example.org/latest.json", missing()).is_err());
        assert!(published_result(&format!("{FEEDS}/stable/latest.json"), Err(ureq::Error::StatusCode(500).into())).is_err());
    }

    #[test]
    fn the_feed_follows_the_channel() {
        // SAFETY: no other test reads or writes this variable.
        unsafe { std::env::remove_var("FURNISH_UPDATE_FEED") };
        assert_eq!(feed_url(Channel::Alpha), "https://furnish-self-service.github.io/furnish/alpha/latest.json");
        assert_eq!(feed_url(Channel::Stable), "https://furnish-self-service.github.io/furnish/stable/latest.json");
        assert!(feed_allowed("https://example.org/latest.json"));
        assert!(feed_allowed("http://127.0.0.1:8123/latest.json"));
        assert!(feed_allowed("http://localhost/latest.json"));
        assert!(!feed_allowed("http://example.org/latest.json"));
        assert!(!feed_allowed("http://127.0.0.1.evil.org/latest.json"));
        assert!(!feed_allowed("http://localhost@evil.org/x"));
        assert!(!feed_allowed("file:///tmp/latest.json"));
    }

    #[test]
    fn newer_versions_only() {
        let manifest = |v: &str| Manifest { version: v.into(), notes: Some("n".into()), pub_date: None, platforms: BTreeMap::new() };
        assert_eq!(manifest("0.1.0-alpha.146").available("0.1.0+alpha.145", Channel::Alpha).unwrap().version, "0.1.0+alpha.146");
        assert!(manifest("0.1.0-alpha.145").available("0.1.0+alpha.145", Channel::Alpha).is_none());
        assert!(manifest("0.1.0-alpha.144").available("0.1.0+alpha.145", Channel::Alpha).is_none());
        assert!(manifest("0.1.0-alpha.146").available("0.1.0", Channel::Stable).is_none());
        assert!(manifest("0.1.0-alpha.146").available("0.1.0", Channel::Alpha).is_some());
        assert!(manifest("0.1.0").available("0.2.0-alpha.1", Channel::Stable).is_some());
        assert!(manifest("1.0.0").available("0.1.0", Channel::Alpha).is_none());
        assert_eq!(Channel::selected("1.0.0", Method::Nix, Some(Channel::Alpha)), Channel::Stable);
        assert!(manifest("garbage").available("0.1.0", Channel::Stable).is_none());
        let parsed: Manifest = serde_json::from_str(
            r#"{"version":"0.1.0-alpha.2","pub_date":"2026-11-16T12:00:00Z","platforms":{"windows-x86_64":{"url":"https://x/a.msi","signature":"s"}}}"#,
        )
        .unwrap();
        assert_eq!(parsed.platforms["windows-x86_64"].url, "https://x/a.msi");
    }

    #[test]
    fn install_methods() {
        let none = Probe::default();
        let p = Path::new;
        // Windows: in the folder the MSI recorded, whatever the case and trailing backslash.
        let msi = Probe { msi_dirs: vec![PathBuf::from(r"C:\Program Files\furnish\")], ..Probe::default() };
        assert_eq!(classify(p(r"C:\Program Files\furnish\furnish.exe"), "windows", &msi), Method::Msi);
        assert_eq!(classify(p(r"c:\program files\FURNISH\furnish-gui.exe"), "windows", &msi), Method::Msi);
        assert_eq!(classify(p(r"C:\src\furnish\target\release\furnish.exe"), "windows", &msi), Method::Other);
        assert_eq!(classify(p(r"C:\Program Files\furnish\furnish.exe"), "windows", &none), Method::Other);
        // macOS: an app folder, from Homebrew if its Caskroom has furnish.
        let app = p("/Applications/furnish.app/Contents/MacOS/furnish");
        assert_eq!(classify(app, "macos", &none), Method::App);
        assert_eq!(classify(app, "macos", &Probe { cask: true, ..Probe::default() }), Method::Homebrew);
        assert_eq!(classify(p("/Users/me/src/furnish/target/release/furnish"), "macos", &none), Method::Other);
        assert_eq!(app_bundle(app), Some(PathBuf::from("/Applications/furnish.app")));
        // Linux: the Nix store, the package that has the file, or something else.
        assert_eq!(classify(p("/nix/store/abc-furnish-0.1.0/bin/furnish"), "linux", &none), Method::Nix);
        assert_eq!(classify(p("/usr/bin/furnish"), "linux", &Probe { package: Some(Method::Rpm), ..Probe::default() }), Method::Rpm);
        assert_eq!(classify(p("/usr/bin/furnish"), "linux", &none), Method::Other);
        assert!(Method::Msi.updates_itself() && Method::App.updates_itself());
        assert!(![Method::Homebrew, Method::Deb, Method::Rpm, Method::Nix, Method::Other].iter().any(|m| m.updates_itself()));
    }

    #[test]
    fn install_methods_from_a_fake_system() {
        let root = tempfile::tempdir().unwrap();
        let exe = Path::new("/usr/bin/furnish");
        assert_eq!(classify(exe, "linux", &Probe::gather(exe, "linux", root.path())), Method::Other);
        std::fs::create_dir_all(root.path().join("var/lib/dpkg/info")).unwrap();
        std::fs::write(root.path().join("var/lib/dpkg/info/furnish.list"), "/.\n/usr\n/usr/bin\n/usr/bin/furnish\n/usr/bin/furnish-gui\n").unwrap();
        assert_eq!(classify(exe, "linux", &Probe::gather(exe, "linux", root.path())), Method::Deb);
        assert_eq!(classify(Path::new("/usr/local/bin/furnish"), "linux", &Probe::gather(Path::new("/usr/local/bin/furnish"), "linux", root.path())), Method::Other);

        let app = Path::new("/Applications/furnish.app/Contents/MacOS/furnish");
        assert_eq!(classify(app, "macos", &Probe::gather(app, "macos", root.path())), Method::App);
        std::fs::create_dir_all(root.path().join("opt/homebrew/Caskroom/furnish/0.1.0-alpha.3")).unwrap();
        assert_eq!(classify(app, "macos", &Probe::gather(app, "macos", root.path())), Method::Homebrew);
    }

    #[test]
    fn auto_means_notify_where_a_package_manager_updates() {
        assert_eq!(classify(Path::new("/nix/store/build/bin/furnish"), "macos", &Probe::default()), Method::Nix);
        for mode in [SelfUpdate::Auto, SelfUpdate::Notify, SelfUpdate::Off] {
            assert_eq!(effective(mode, Method::Nix), SelfUpdate::Off);
        }
        assert_eq!(effective(SelfUpdate::Auto, Method::Msi), SelfUpdate::Auto);
        assert_eq!(effective(SelfUpdate::Auto, Method::Homebrew), SelfUpdate::Notify);
        assert_eq!(effective(SelfUpdate::Auto, Method::Other), SelfUpdate::Notify);
        assert_eq!(effective(SelfUpdate::Notify, Method::App), SelfUpdate::Notify);
        assert_eq!(effective(SelfUpdate::Off, Method::Msi), SelfUpdate::Off);
    }

    #[test]
    fn the_public_key_is_the_one_the_desktop_app_has() {
        let conf: serde_json::Value = serde_json::from_str(include_str!("../gui/src-tauri/tauri.conf.json")).unwrap();
        assert_eq!(conf["plugins"]["updater"]["pubkey"], PUBLIC_KEY);
        // And it decodes as a minisign key.
        use base64::Engine as _;
        let text = String::from_utf8(base64::engine::general_purpose::STANDARD.decode(PUBLIC_KEY).unwrap()).unwrap();
        minisign_verify::PublicKey::decode(&text).unwrap();
    }

    /// A signature made with `tauri signer sign --app-version 1.2.3` over "hello\n", with a
    /// throwaway key (not furnish's).
    const TEST_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDU1Q0Y2RDk5NUM3MjMyRDMKUldUVE1uSmNtVzNQVlhra0piOTU5aHVyYXcwZWJXSkVpWUhMczhVejJRR1VYSEFSZ0JncG1rd2YK";
    const TEST_SIG: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVUVE1uSmNtVzNQVmNrQXduZW1HdzFmdjhxbHZpRG9URjh2WUVyVkJiUGp5ZENXbWVYdzdadXZjNHRFK0JoUFd3Y0lGajhzMXArYWxJVDU0WTVOVUViSE9tMG4zc09CZndzPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzk0ODU2NzIzCWZpbGU6dC50eHQJdmVyc2lvbjoxLjIuMwpjU1UraEpTalg4Tm1rVmowVUh1QmpDUHhkRjhJZXZUM2RyR21kcTZxVmZTNDZHNmNVT21reWVyVm5MSXN1Y1pGeWhLaUE0dFVJT3hFUUVwbS9VYnFCdz09Cg==";

    #[test]
    fn downloads_are_checked_against_the_key_and_the_signed_version() {
        verify(b"hello\n", TEST_SIG, TEST_KEY, "1.2.3").unwrap();
        verify(b"hello\n", TEST_SIG, TEST_KEY, "v1.2.3").unwrap();
        let err = verify(b"hello!\n", TEST_SIG, TEST_KEY, "1.2.3").unwrap_err();
        assert!(format!("{err:#}").contains("doesn't match"), "{err:#}");
        let err = verify(b"hello\n", TEST_SIG, TEST_KEY, "1.2.4").unwrap_err();
        assert!(format!("{err:#}").contains("for version 1.2.3"), "{err:#}");
        assert!(verify(b"hello\n", TEST_SIG, PUBLIC_KEY, "1.2.3").is_err());
    }
}
