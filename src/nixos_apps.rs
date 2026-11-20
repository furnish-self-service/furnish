//! Furnish/NixOS: apps that go into the computer's NixOS configuration, a flake, instead of a
//! profile. furnish never edits the configuration's files. Its flake has an input `apps`, which
//! the configuration points at furnish's local flake once, by hand:
//!
//! ```nix
//! inputs.furnish.inputs.apps.url = "path:/var/lib/furnish/apps";
//! ```
//!
//! furnish's NixOS module adds what that flake lists (`apps.json`): packages from the system's
//! nixpkgs, and NixOS options. Administrators can restrict option names with `services.furnish.nix.allowedOptions`.
//!
//! Installing (`set`), only in the furnish service:
//!
//! 1. furnish writes the app into `apps.json`;
//! 2. `nix flake update furnish/apps` changes only that entry of the configuration's flake.lock;
//! 3. `nix build` builds the system, `switch-to-configuration` switches to it (or `boot`);
//! 4. furnish commits flake.lock if the configuration's repository had no other changes.
//!
//! When building fails, furnish puts apps.json and flake.lock back.
//!
//! Home Manager (`Place`). With furnish's Home Manager module set up for a person, apps go into
//! their Home Manager configuration instead, unless the setting `osSpecific.nixos.installInto`
//! says `system` (`install_into`). The app's `homeOptions` replace its `options` there.
//!
//! - Standalone Home Manager: furnish's local flake is in the person's data folder
//!   (`home_apps_dir`), which their configuration's furnish/home-apps input points at. Everything
//!   runs as the person: furnish writes apps.json, updates furnish/home-apps, builds the
//!   generation and activates it, also when the furnish service runs the resource for them.
//! - Home Manager as a NixOS module: the person's apps go under `homes.<user>` in the system's
//!   apps.json, and the service rebuilds the system as above. The allowlist comes from the NixOS
//!   configuration (/etc/furnish/nixos.json), not from the person's home folder.
//!
//! Security. A rebuild runs as root what the configuration says. A configuration that belongs to
//! a user is rebuilt only when that user (or root) asks, never on behalf of someone else, and
//! never if anyone else can change it or a folder above it. A group whose members may become
//! root anyway may (`services.furnish.nix.trustedGroups`, by default `wheel` where sudo or doas
//! let it run anything as root). Steps 2, 3 and 4 run as the folder's
//! owner, not as root: then no root process opens a git repository another account controls
//! (git's hooks and config would run as root; libgit2 in Nix refuses such a repository), and
//! furnish reads and writes the owner's files only with the owner's rights. Only the switch runs
//! as root. A folder of root's whose git folder belongs to a member of a trusted group (an
//! admin's clone in a shared /etc/nixos) is the exception: root runs git and Nix in it, allowed
//! with `safe.directory` for just that repository (`GitTrust`, `may_trust_git`).

use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

mod external_options;

/// furnish's local flake with the apps it installed.
pub const APPS_DIR: &str = "/var/lib/furnish/apps";
/// What the configuration's furnish/apps input points at.
pub const INPUT_URL: &str = "path:/var/lib/furnish/apps";
/// The line to add to the configuration's flake.nix, with the follows line furnish recommends.
pub const SETUP_LINES: [&str; 2] =
    ["inputs.furnish.inputs.nixpkgs.follows = \"nixpkgs\";", "inputs.furnish.inputs.apps.url = \"path:/var/lib/furnish/apps\";"];
/// The local flake's flake.nix (the NixOS module writes the same, `appsFlake`).
const APPS_FLAKE: &str = "{ outputs = _: { furnish = builtins.fromJSON (builtins.readFile ./apps.json); }; }\n";
const FACTS: &str = "/etc/furnish/nixos.json";
/// furnish's input for the apps it installs into a standalone Home Manager configuration.
pub const HOME_INPUT: &str = "furnish/home-apps";

/// Where furnish suggests its local flake for this person's standalone Home Manager
/// configuration: `furnish/home-apps` in `$XDG_DATA_HOME`, by default `~/.local/share`, also on
/// macOS (a flake input can't have the space of `~/Library/Application Support`). Once the
/// configuration points its input somewhere, furnish uses that folder (`configured_home_apps`).
pub fn home_apps_dir() -> PathBuf {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".local/share"));
    data.join("furnish/home-apps")
}

/// The folder a Home Manager configuration's furnish/home-apps input points at: as flake.lock
/// has it (what Nix uses), else as flake.nix says. The service takes it from here, because it
/// can't see the person's environment (`$XDG_DATA_HOME`).
pub fn configured_home_apps(lock: Option<&str>, flake: &str) -> Option<PathBuf> {
    let from_lock = lock.and_then(|l| serde_json::from_str::<Value>(l).ok()).and_then(|l| {
        let nodes = &l["nodes"];
        let node = |v: &Value| v.as_str().map(str::to_string);
        let furnish = node(&nodes[l["root"].as_str()?]["inputs"]["furnish"])?;
        let apps = node(&nodes[&furnish]["inputs"]["home-apps"])?;
        let original = &nodes[&apps]["original"];
        (original["type"] == "path").then(|| original["path"].as_str().map(PathBuf::from)).flatten()
    });
    from_lock.or_else(|| {
        let re = regex::Regex::new(r#"home-apps\.url\s*=\s*"path:([^"]+)""#).ok()?;
        re.captures(flake).map(|c| PathBuf::from(&c[1]))
    })
    .filter(|p| p.is_absolute())
}

/// The line for the Home Manager configuration's flake.nix that points furnish/home-apps there.
pub fn home_setup_line(dir: &Path) -> String {
    format!("inputs.furnish.inputs.home-apps.url = \"path:{}\";", dir.display())
}

/// The URL in a setup line, which the configuration's flake.nix has to contain.
pub fn line_url(line: &str) -> &str {
    line.split('"').nth(1).unwrap_or(line)
}

/// Where apps go: `system` or `home`. The setting if there is one; else `home` when furnish's
/// Home Manager module is set up for the person.
pub fn install_into(setting: Option<&crate::settings::Leaf>, home_module: bool) -> &'static str {
    match setting.and_then(|l| l.value.as_str()) {
        Some("system") => "system",
        Some("home") => "home",
        _ if home_module => "home",
        _ => "system",
    }
}

/// One app's part of the configuration.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Entry {
    /// Attribute names in the system's nixpkgs, e.g. `htop` or `kdePackages.kdenlive`.
    #[serde(default)]
    pub packages: Vec<String>,
    /// NixOS options by name, e.g. `programs.htop.enable`: true.
    #[serde(default)]
    pub options: BTreeMap<String, Value>,
}

/// `apps.json`: the apps furnish installed, by the resource's `name`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Apps {
    pub version: u32,
    #[serde(default)]
    pub apps: BTreeMap<String, Entry>,
    /// Home Manager as a NixOS module: each person's apps, by user name (the system's apps.json).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub homes: BTreeMap<String, Home>,
}

/// One person's apps in the system's apps.json.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Home {
    #[serde(default)]
    pub apps: BTreeMap<String, Entry>,
}

impl Default for Apps {
    fn default() -> Self {
        Apps { version: 1, apps: BTreeMap::new(), homes: BTreeMap::new() }
    }
}

/// Whether an entry of allowedOptions allows an option: the same parts, `*` for any one part.
pub fn allowed(pattern: &str, option: &str) -> bool {
    let (p, o): (Vec<_>, Vec<_>) = (pattern.split('.').collect(), option.split('.').collect());
    p.len() == o.len() && p.iter().zip(&o).all(|(p, o)| *p == "*" || p == o)
}

/// A name of plain parts separated by dots, as nixpkgs attributes and NixOS options have.
fn plain_name(s: &str, extra: &str) -> bool {
    !s.is_empty()
        && s.split('.').all(|part| {
            part.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && part.chars().all(|c| c.is_ascii_alphanumeric() || "_-".contains(c) || extra.contains(c))
        })
}

fn explicit_name(name: &str) -> bool {
    plain_name(name, "") && !name.contains('.')
}

// A separate namespace from Furnish/NixOS names, including for dotted Nix attributes.
const PACKAGE: &str = "furnishNixPackage";
/// The client reads the shared profile when the service would perform the installation.
pub(crate) const SERVICE_PROFILE: &str = "furnishNixServiceProfile";

fn package_key(name: &str) -> bool {
    let mut parts = name.split(':');
    let digest = |s: &str| s.len() == 43 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b));
    parts.next() == Some("package") && parts.next().is_some_and(digest) && parts.next().is_some_and(digest) && parts.next().is_none()
}

/// Stamp stable ownership before the engine splits the document or strips version constraints.
/// The collection may name a step, but cannot provide another app's ownership key.
pub(crate) fn stamp_packages(doc: &mut Value, app: &str) -> Result<()> {
    fn component(bytes: &[u8]) -> String {
        use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
        use sha2::{Digest, Sha256};
        URL_SAFE_NO_PAD.encode(Sha256::digest(bytes))
    }
    fn visit(resources: &mut Value, app_hash: &str, path: &mut Vec<String>, removal: bool) -> Result<()> {
        for r in resources.as_array_mut().into_iter().flatten() {
            path.push(r["name"].as_str().unwrap_or_default().to_owned());
            if r["type"] == crate::catalog::PACKAGE_TYPE {
                let step_hash = component(&serde_json::to_vec(path)?);
                anyhow::ensure!(r["metadata"].is_null() || r["metadata"].is_object(), "invalid metadata on package step {path:?}");
                let versioned = r["properties"].get(crate::required::PROPERTY).is_some();
                if let Some(metadata) = r["metadata"].as_object_mut() {
                    metadata.remove(PACKAGE_FINGERPRINT);
                }
                r["metadata"][PACKAGE] = json!({
                    "key": format!("package:{app_hash}:{step_hash}"), "versioned": versioned, "removal": removal
                });
            }
            if let Some(children) = r["properties"].get_mut("resources") {
                visit(children, app_hash, path, removal)?;
            }
            path.pop();
        }
        Ok(())
    }
    let hash = component(app.as_bytes());
    visit(&mut doc["resources"], &hash, &mut Vec::new(), false)?;
    if let Some(steps) = doc.pointer_mut("/metadata/furnish/uninstall") {
        visit(steps, &hash, &mut Vec::new(), true)?;
    }
    Ok(())
}

/// Checks an app's entry: package and option names, and each option against the allowlist.
pub fn check(name: &str, entry: &Entry, allowlist: &[String]) -> Result<()> {
    anyhow::ensure!(explicit_name(name), "invalid Furnish/NixOS resource name '{name}'");
    check_in(name, entry, Some(allowlist), Place::System)
}

fn check_in(name: &str, entry: &Entry, allowlist: Option<&[String]>, place: Place) -> Result<()> {
    if !(explicit_name(name) || package_key(name) && entry.packages.len() == 1 && entry.options.is_empty()) {
        bail!("'{name}' isn't a name for a Furnish/NixOS resource (letters, digits, _ and -)");
    }
    for p in &entry.packages {
        if !plain_name(p, "+'") {
            bail!("'{p}' isn't a package's name in nixpkgs");
        }
    }
    for o in entry.options.keys() {
        if !plain_name(o, "") {
            bail!("'{o}' isn't an option's name");
        }
        if allowlist.is_some_and(|list| !list.iter().any(|a| allowed(a, o))) {
            match place {
                Place::System => bail!("the app sets the NixOS option {o}, which isn't in services.furnish.nix.allowedOptions on this computer"),
                _ => bail!("the app sets the Home Manager option {o}, which isn't in services.furnish.nix.allowedOptions in your Home Manager configuration"),
            }
        }
    }
    Ok(())
}

/// What the resource gets and returns (DSC's JSON on stdin and stdout).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Desired {
    name: String,
    #[serde(default)]
    packages: Vec<String>,
    #[serde(default)]
    options: BTreeMap<String, Value>,
    #[serde(default)]
    home_options: BTreeMap<String, Value>,
    /// `ONLY_IF_SET_UP`, which the engine handles.
    #[serde(default, rename = "onlyIfSetUp")]
    _only_if_set_up: bool,
    #[serde(rename = "_exist", default = "yes")]
    exist: bool,
}

fn yes() -> bool {
    true
}

impl Desired {
    /// The app's entry in the apps.json of that place: its options for there.
    fn entry(&self, place: Place) -> Entry {
        let options = if place == Place::System { &self.options } else { &self.home_options };
        Entry { packages: self.packages.clone(), options: options.clone() }
    }
}

/// Where an app goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// The NixOS configuration (`apps` of the system's apps.json).
    System,
    /// Home Manager as a NixOS module (`homes.<user>` of the system's apps.json).
    HomeNixos,
    /// Standalone Home Manager (the person's own apps.json, `home_apps_dir`).
    HomeStandalone,
}

impl Place {
    fn name(self) -> &'static str {
        if self == Place::System { "system" } else { "home" }
    }
    fn what(self) -> &'static str {
        if self == Place::System { "the NixOS configuration" } else { "your Home Manager configuration" }
    }
}

/// A tool or a part of the setup is missing: the app shows "Needs …" instead of failing.
#[derive(Debug)]
struct Missing(String);

impl std::fmt::Display for Missing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "needs {}", self.0)
    }
}

impl std::error::Error for Missing {}

fn missing(what: impl Into<String>) -> anyhow::Error {
    Missing(what.into()).into()
}

/// What furnish's NixOS module wrote (`facts` in flake.nix).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Facts {
    nix: FactsNix,
    home_manager_users: BTreeMap<String, crate::nixos::HomeUser>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct FactsNix {
    allowed_options: Option<Vec<String>>,
    rebuild: Option<String>,
    /// Groups whose members may become root anyway (`services.furnish.nix.trustedGroups`): a
    /// configuration folder they can write to is still rebuilt (`may_rebuild`).
    trusted_groups: Vec<String>,
}

/// The configuration to install into, as furnish finds it now; in the service (and the resource
/// it runs) from the policy and root's places only, never from a user's settings.
struct Target {
    dir: PathBuf,
    attribute: String,
    facts: Facts,
}

fn in_service() -> bool {
    std::env::var("FURNISH_SERVICE").as_deref() == Ok("1") && is_root()
}

impl Target {
    fn here() -> Result<Target> {
        let facts: Facts = match std::fs::read_to_string(FACTS) {
            Ok(text) => serde_json::from_str(&text).with_context(|| format!("reading {FACTS}"))?,
            Err(_) => return Err(missing("furnish's NixOS module (services.furnish.enable in the NixOS configuration)")),
        };
        let setup = crate::nixos::detect(&crate::nixos::Context::new(in_service()));
        let Some(setup) = setup else { return Err(missing("NixOS")) };
        let Some(found) = setup.configuration else {
            return Err(missing("the folder of your NixOS configuration (services.furnish.nix.configuration)"));
        };
        if found.kind != crate::nixos::Kind::Flake {
            return Err(missing(format!("a NixOS configuration that is a flake ({} has no flake.nix)", found.path.display())));
        }
        if !setup.apps_input {
            return Err(missing(format!("the line {} in {}", SETUP_LINES[1], found.file.display())));
        }
        Ok(Target { dir: found.path, attribute: setup.flake_attribute.value, facts })
    }
}

impl Target {
    fn home_enabled(&self, person: &Account) -> bool {
        self.facts.home_manager_users.get(&person.name).is_some_and(|user| user.enable)
    }

    /// The options apps may set there: the NixOS module's, or the person's from Home Manager's
    /// NixOS module, as the NixOS configuration says.
    fn allowlist(&self, place: Place, user: &str) -> Option<Vec<String>> {
        match place {
            Place::HomeNixos => self.facts.home_manager_users.get(user).and_then(|u| u.allowed_options.clone()),
            _ => self.facts.nix.allowed_options.clone(),
        }
    }
}

/// A person's standalone Home Manager configuration, ready for furnish to install into.
struct HomeTarget {
    dir: PathBuf,
    attribute: String,
    allowlist: Option<Vec<String>>,
    apps_dir: PathBuf,
}

/// Reads a file of the person's: with their rights when furnish runs as root.
fn read_as(person: &Account, path: &Path) -> Option<String> {
    if is_root() && person.uid != 0 {
        owner_read(person, path).ok().flatten().and_then(|b| String::from_utf8(b).ok())
    } else {
        std::fs::read_to_string(path).ok()
    }
}

/// What furnish's Home Manager module wrote into the person's home (`None`: not set up).
fn home_facts(person: &Account) -> Option<crate::nixos::HomeFacts> {
    serde_json::from_str(&read_as(person, &person.home.join(crate::nixos::HOME_FACTS))?).ok()
}

/// `osSpecific.nixos.installInto` as this user's settings and the policy give it, for a request
/// to the service (`service::Request::Install`).
pub fn own_install_into() -> Option<String> {
    let leaf = crate::nixos::chosen(&crate::policy::load(), &crate::settings::user_choices()).install_into?;
    leaf.value.as_str().map(str::to_string)
}

/// The person's `osSpecific.nixos.installInto`, or the policy's. In the service, the caller sent
/// theirs with the request (`service::REQUESTER_INSTALL_INTO`): their settings may be anywhere.
/// It only chooses between places they may install into anyway.
fn install_into_for(_person: &Account, module: bool) -> &'static str {
    let policy = crate::policy::load();
    let leaf = if in_service() {
        let theirs = std::env::var(crate::service::REQUESTER_INSTALL_INTO).ok().filter(|v| !v.is_empty());
        let mut choices = serde_json::Map::new();
        if let Some(v) = theirs {
            choices.insert("osSpecific".into(), json!({ "nixos": { "installInto": v } }));
        }
        // The policy's lock still wins, as for settings people make themselves.
        crate::nixos::chosen(&policy, &choices).install_into
    } else {
        crate::nixos::chosen(&policy, &crate::settings::user_choices()).install_into
    };
    install_into(leaf.as_ref(), module)
}

impl HomeTarget {
    fn find(person: &Account) -> Result<HomeTarget> {
        let Some(facts) = home_facts(person) else {
            return Err(missing("furnish's Home Manager module (services.furnish.enable in your Home Manager configuration)"));
        };
        Self::from_facts(person, facts)
    }

    fn from_facts(person: &Account, facts: crate::nixos::HomeFacts) -> Result<HomeTarget> {
        let dir = home_dir_of(person, &facts);
        let Some(flake) = read_as(person, &dir.join("flake.nix")) else {
            return Err(missing(format!("a Home Manager configuration that is a flake ({} has no flake.nix)", dir.display())));
        };
        let Some(apps_dir) = configured_home_apps(read_as(person, &dir.join("flake.lock")).as_deref(), &flake) else {
            let suggested = if in_service() { person.home.join(".local/share/furnish/home-apps") } else { home_apps_dir() };
            return Err(missing(format!("the line {} in {}", home_setup_line(&suggested), dir.join("flake.nix").display())));
        };
        let attribute = facts.nix.flake_attribute.clone().unwrap_or_else(|| person.name.clone());
        Ok(HomeTarget { dir, attribute, allowlist: facts.nix.allowed_options, apps_dir })
    }
}

/// The folder of the person's Home Manager configuration: from furnish's Home Manager module,
/// else (outside the service) as furnish finds it, else `~/.config/home-manager`.
fn home_dir_of(person: &Account, facts: &crate::nixos::HomeFacts) -> PathBuf {
    match &facts.nix.configuration {
        Some(c) => PathBuf::from(c),
        None if !in_service() => crate::nixos::detect(&crate::nixos::Context::here())
            .and_then(|s| s.home_manager_configuration)
            .map_or_else(|| person.home.join(".config/home-manager"), |f| f.path),
        None => person.home.join(".config/home-manager"),
    }
}

/// The person furnish acts for: in the service, who asked; else who runs furnish.
fn person() -> Result<Account> {
    if in_service() {
        let uid = std::env::var("FURNISH_REQUESTER_UID").ok().and_then(|v| v.parse().ok())
            .context("the furnish service doesn't know who asked")?;
        return lookup(uid).with_context(|| format!("no account with the requester's id {uid}"));
    }
    let uid = current_uid();
    Ok(lookup(uid).unwrap_or_else(|| Account {
        uid,
        gid: uid,
        name: std::env::var("USER").unwrap_or_default(),
        home: dirs::home_dir().unwrap_or_default(),
    }))
}

/// Profile locations for the current account must come from the OS, even if HOME or USER
/// was overridden by the desktop launcher.
#[cfg(unix)]
pub fn current_account() -> Option<Account> {
    lookup(current_uid())
}

/// An account from the system's user database (also macOS's directory service and NSS), else
/// from /etc/passwd.
fn lookup(uid: u32) -> Option<Account> {
    #[cfg(unix)]
    {
        let mut buf = vec![0 as libc::c_char; 16 * 1024];
        // SAFETY: zeroed passwd is a valid out-parameter; getpwuid_r fills it with pointers into buf.
        let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
        let mut found: *mut libc::passwd = std::ptr::null_mut();
        let rc = unsafe { libc::getpwuid_r(uid, &mut pw, buf.as_mut_ptr(), buf.len(), &mut found) };
        if rc == 0 && !found.is_null() {
            // SAFETY: pw_name and pw_dir point to NUL-terminated strings in buf.
            let text = |p: *const libc::c_char| unsafe { std::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned();
            return Some(Account { uid, gid: pw.pw_gid, name: text(pw.pw_name), home: PathBuf::from(text(pw.pw_dir)) });
        }
    }
    account(&std::fs::read_to_string("/etc/passwd").unwrap_or_default(), uid)
}

#[cfg(unix)]
fn current_uid() -> u32 {
    // SAFETY: getuid has no preconditions.
    unsafe { libc::getuid() }
}

#[cfg(not(unix))]
fn current_uid() -> u32 {
    u32::MAX
}

/// Reads a person's own apps.json, with their rights.
fn read_apps_as(person: &Account, dir: &Path) -> Result<Apps> {
    match read_as(person, &dir.join("apps.json")) {
        Some(text) => serde_json::from_str(&text).with_context(|| format!("reading {}", dir.join("apps.json").display())),
        None => Ok(Apps::default()),
    }
}

/// Writes a person's own local flake, with their rights.
fn write_apps_as(person: &Account, dir: &Path, apps: &Apps) -> Result<()> {
    if !(is_root() && person.uid != 0) {
        return write_apps(dir, apps);
    }
    let mut mkdir = as_owner(person, "mkdir");
    mkdir.arg("-p").arg("--").arg(dir);
    run(mkdir, "creating furnish's local flake")?;
    owner_write(person, &dir.join("flake.nix"), APPS_FLAKE.as_bytes())?;
    owner_write(person, &dir.join("apps.json"), (serde_json::to_string_pretty(apps)? + "\n").as_bytes())
}

/// Creates furnish's local flake for this person's standalone Home Manager configuration, empty,
/// so the configuration's first `nix flake lock` finds it.
pub fn ensure_home_apps(dir: &Path) -> Result<()> {
    if dir.join("apps.json").exists() { Ok(()) } else { write_apps(dir, &Apps::default()) }
}

/// Where the app is now, if anywhere: the system's apps, the person's part of them, or the
/// person's own apps.
fn find(name: &str, person: &Account) -> Result<Option<(Place, Entry)>> {
    Ok(find_entry(person, |apps| Ok(apps.get_key_value(name)))?.map(|(place, _, entry)| (place, entry)))
}

fn find_entry(
    person: &Account, lookup: impl Fn(&BTreeMap<String, Entry>) -> Result<Option<(&String, &Entry)>>,
) -> Result<Option<(Place, String, Entry)>> {
    let system = read_apps(Path::new(APPS_DIR))?;
    if let Some((k, e)) = lookup(&system.apps)? {
        return Ok(Some((Place::System, k.clone(), e.clone())));
    }
    if let Some(home) = system.homes.get(&person.name)
        && let Some((k, e)) = lookup(&home.apps)?
    {
        return Ok(Some((Place::HomeNixos, k.clone(), e.clone())));
    }
    let Some(dir) = own_apps_dir(person) else { return Ok(None) };
    let own = read_apps_as(person, &dir)?;
    Ok(lookup(&own.apps)?.map(|(k, e)| (Place::HomeStandalone, k.clone(), e.clone())))
}

/// The person's own local flake, where their configuration points (`None`: it points nowhere).
fn own_apps_dir(person: &Account) -> Option<PathBuf> {
    let dir = home_dir_of(person, &home_facts(person).unwrap_or_default());
    configured_home_apps(read_as(person, &dir.join("flake.lock")).as_deref(), &read_as(person, &dir.join("flake.nix"))?)
}

/// Destinations configured for a fresh install, independently of the preferred destination.
#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct NixSetup {
    pub system: bool,
    pub home: bool,
}

pub(crate) fn nix_setup() -> NixSetup {
    if cfg!(windows) || !crate::nixos::nix_installed() {
        return NixSetup::default();
    }
    let system = cfg!(target_os = "linux").then(Target::here).and_then(Result::ok);
    // Home Manager belongs to the requesting account, including when a service runs as root.
    let home = person().ok().is_some_and(|person| {
        home_facts(&person).is_some_and(|facts| {
            if facts.nixos_module {
                system.as_ref().is_some_and(|target| target.home_enabled(&person))
            } else {
                HomeTarget::from_facts(&person, facts).is_ok()
            }
        })
    });
    NixSetup { system: system.is_some(), home }
}

/// Whether furnish can install into a place: `Missing` says what is missing.
fn ready(place: Place, person: &Account) -> Result<()> {
    match place {
        Place::System => Target::here().map(drop),
        Place::HomeNixos => {
            let target = Target::here()?;
            if !target.home_enabled(person) {
                return Err(missing(format!(
                    "furnish's Home Manager module for {0} (home-manager.users.{0}.services.furnish.enable in the NixOS configuration)",
                    person.name
                )));
            }
            Ok(())
        }
        Place::HomeStandalone => HomeTarget::find(person).map(drop),
    }
}

/// Where to install an app: `install_into`, unless the app has options only for the other place,
/// then there.
fn choose(desired: &Desired, person: &Account) -> Result<Place> {
    let place = place_for(desired, person);
    ready(place, person)?;
    Ok(place)
}

fn place_for(desired: &Desired, person: &Account) -> Place {
    let facts = home_facts(person);
    let home = if facts.as_ref().is_some_and(|f| f.nixos_module) { Place::HomeNixos } else { Place::HomeStandalone };
    let wanted = if install_into_for(person, facts.is_some()) == "home" { home } else { Place::System };
    pick(wanted, home, desired)
}

/// The property that makes a Furnish/NixOS step apply only where furnish can install it: where
/// its place isn't set up, furnish skips the step, as for a false `condition` (`not_needed_here`).
pub const ONLY_IF_SET_UP: &str = "onlyIfSetUp";

/// Why a step is skipped for `ONLY_IF_SET_UP`, as the preview shows it ("Not needed here: …").
pub const NO_NIX: &str = "Nix isn't installed on this computer";
pub const NO_HOME: &str = "Home Manager isn't set up for you";
pub const NO_SYSTEM: &str = "the NixOS configuration isn't set up for furnish";

pub(crate) fn only_if_set_up(resource: &Value) -> bool {
    resource["type"] == "Furnish/NixOS" && resource["properties"][ONLY_IF_SET_UP] == true
}

/// For a Furnish/NixOS step with `onlyIfSetUp: true`: why it doesn't apply on this computer, or
/// `None` when it applies. Decided before DSC runs, because elsewhere the resource may not exist.
/// A step whose app is already in a local flake applies, so it can be removed.
pub fn not_needed_here(resource: &Value) -> Option<&'static str> {
    not_needed(resource, crate::nixos::nix_installed(), |desired| {
        let person = person().ok()?;
        if find(&desired.name, &person).ok().flatten().is_some() {
            return None;
        }
        let place = place_for(desired, &person);
        ready(place, &person).err().filter(|e| e.is::<Missing>()).map(|_| place)
    })
}

/// `not_needed_here` without the computer: `unready` gives the place the app would go to if that
/// isn't set up.
fn not_needed(resource: &Value, nix: bool, unready: impl FnOnce(&Desired) -> Option<Place>) -> Option<&'static str> {
    if !only_if_set_up(resource) {
        return None;
    }
    if !nix {
        return Some(NO_NIX);
    }
    let desired: Desired = serde_json::from_value(resource["properties"].clone()).ok()?;
    unready(&desired).map(|place| if place == Place::System { NO_SYSTEM } else { NO_HOME })
}

/// The place an app goes: the wanted one, unless the app sets options only for the other.
fn pick(wanted: Place, home: Place, desired: &Desired) -> Place {
    let (system_only, home_only) = (desired.home_options.is_empty() && !desired.options.is_empty(), desired.options.is_empty() && !desired.home_options.is_empty());
    match wanted {
        Place::System if home_only => home,
        Place::HomeNixos | Place::HomeStandalone if system_only => Place::System,
        p => p,
    }
}

/// A fresh target profile has no installed entries. Use the same default destination and
/// option-driven selection as a fresh machine, without inspecting this computer's settings.
pub(crate) fn missing_profile_destination(properties: &Value, system: bool, home: bool) -> Result<Option<&'static str>> {
    let desired: Desired = serde_json::from_value(properties.clone())?;
    let wanted = if install_into(None, home) == "home" { Place::HomeStandalone } else { Place::System };
    Ok(match pick(wanted, Place::HomeStandalone, &desired) {
        Place::System if !system => Some("the NixOS configuration is not set up in this profile"),
        Place::HomeStandalone | Place::HomeNixos if !home => Some("Home Manager is not set up in this profile"),
        _ => None,
    })
}

/// Install (or remove) an app in the person's standalone Home Manager configuration: as the
/// person, update furnish/home-apps, build the generation and activate it.
fn apply_home(home: &HomeTarget, desired: &Desired, person: &Account) -> Result<()> {
    let entry = desired.entry(Place::HomeStandalone);
    if desired.exist {
        check_in(&desired.name, &entry, home.allowlist.as_deref(), Place::HomeStandalone)?;
    }
    let before = read_apps_as(person, &home.apps_dir)?;
    let mut after = before.clone();
    if desired.exist {
        after.apps.insert(desired.name.clone(), entry);
    } else {
        after.apps.remove(&desired.name);
    }
    let dir = &home.dir;
    let lock = dir.join("flake.lock");
    let mut status = as_owner(person, "git");
    status.arg("-C").arg(dir).args(["status", "--porcelain"]);
    let clean = run(status, "git status").is_ok_and(|s| s.is_empty());
    let old_lock = read_as(person, &lock);
    write_apps_as(person, &home.apps_dir, &after)?;
    let undo = |why: anyhow::Error| -> anyhow::Error {
        let mut problems = Vec::new();
        if let Err(e) = write_apps_as(person, &home.apps_dir, &before) {
            problems.push(format!("putting back {}: {e:#}", home.apps_dir.display()));
        }
        if let Some(old) = &old_lock
            && let Err(e) = owner_write(person, &lock, old.as_bytes())
        {
            problems.push(format!("putting back {}: {e:#}", lock.display()));
        }
        if problems.is_empty() { why } else { why.context(problems.join("; ")) }
    };

    info(&format!("Updating {HOME_INPUT} in {}", lock.display()));
    let mut update = nix(person);
    update.args(["flake", "update", HOME_INPUT, "--flake"]).arg(dir);
    run(update, &format!("nix flake update {HOME_INPUT}")).map_err(&undo)?;

    info(&format!("Building the Home Manager generation from {}", dir.display()));
    let mut build = nix(person);
    build.args(["build", "--no-link", "--print-out-paths"]).arg(format!(
        "{}#homeConfigurations.\"{}\".activationPackage",
        dir.display(),
        home.attribute
    ));
    let generation = run(build, "building the Home Manager generation").map_err(&undo)?;
    if !generation.starts_with("/nix/store/") || generation.contains(char::is_whitespace) {
        return Err(undo(anyhow::anyhow!("building the generation gave no store path: {generation}")));
    }

    // What `home-manager switch` does after building.
    info(&format!("Activating {generation}"));
    run(as_owner(person, format!("{generation}/activate")), "activating the Home Manager generation")?;
    commit(as_owner(person, "git"), dir, clean, desired);
    Ok(())
}

/// Commits flake.lock if the repository had no other changes, with `git` as the configuration's
/// owner.
fn commit(mut commit: Command, dir: &Path, clean: bool, desired: &Desired) {
    if clean {
        let verb = if desired.exist { "install" } else { "uninstall" };
        let label = if package_key(&desired.name) { desired.packages.first().unwrap_or(&desired.name) } else { &desired.name };
        commit.arg("-C").arg(dir).args(["commit", "-q", "-m", &format!("furnish: {verb} {label}"), "--", "flake.lock"]);
        match run(commit, "git commit") {
            Ok(_) => info(&format!("Committed flake.lock in {}", dir.display())),
            Err(e) => eprintln!("{}", json!({ "warn": format!("flake.lock changed, commit it yourself: {e:#}") })),
        }
    } else {
        info(&format!("{} has other changes: furnish leaves committing flake.lock to you", dir.display()));
    }
}

/// Whether the configuration's flake.nix points furnish/apps at the local flake. A text check, as
/// in `nixos::check`: it reads one file and evaluates nothing.
pub fn has_input(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("flake.nix")).is_ok_and(|t| t.contains(INPUT_URL))
}

/// The local flake's apps (none if it doesn't exist yet).
pub fn read_apps(dir: &Path) -> Result<Apps> {
    match std::fs::read_to_string(dir.join("apps.json")) {
        Ok(text) => serde_json::from_str(&text).with_context(|| format!("reading {}", dir.join("apps.json").display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Apps::default()),
        Err(e) => Err(e).with_context(|| format!("reading {}", dir.join("apps.json").display())),
    }
}

/// Writes the local flake: flake.nix and apps.json, each replaced in one step.
pub fn write_apps(dir: &Path, apps: &Apps) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    for (file, text) in [("flake.nix", APPS_FLAKE.to_string()), ("apps.json", serde_json::to_string_pretty(apps)? + "\n")] {
        let tmp = dir.join(format!(".{file}.tmp"));
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, dir.join(file)).with_context(|| format!("writing {}", dir.join(file).display()))?;
    }
    Ok(())
}

/// The account that owns the configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct Account {
    pub uid: u32,
    pub gid: u32,
    pub name: String,
    pub home: PathBuf,
}

/// An account from /etc/passwd (NixOS keeps its users there).
fn account(passwd: &str, uid: u32) -> Option<Account> {
    passwd.lines().find_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        (f.len() >= 7 && f[2].parse::<u32>().ok()? == uid).then(|| Account {
            uid,
            gid: f[3].parse().unwrap_or(uid),
            name: f[0].to_string(),
            home: PathBuf::from(f[5]),
        })
    })
}

/// A folder and each folder above it: path, owner (uid), group (gid) and mode.
pub type Chain = Vec<(PathBuf, u32, u32, u32)>;

#[cfg(unix)]
fn chain(dir: &Path) -> Result<Chain> {
    use std::os::unix::fs::MetadataExt;
    dir.ancestors()
        .map(|d| {
            let m = std::fs::symlink_metadata(d).with_context(|| format!("reading {}", d.display()))?;
            Ok((d.to_path_buf(), m.uid(), m.gid(), m.mode()))
        })
        .collect()
}

/// A group's name from the system's group database (`None`: no such group).
#[cfg(unix)]
fn group_name(gid: u32) -> Option<String> {
    let mut buf = vec![0 as libc::c_char; 16 * 1024];
    // SAFETY: zeroed group is a valid out-parameter; getgrgid_r fills it with pointers into buf.
    let mut gr: libc::group = unsafe { std::mem::zeroed() };
    let mut found: *mut libc::group = std::ptr::null_mut();
    let rc = unsafe { libc::getgrgid_r(gid, &mut gr, buf.as_mut_ptr(), buf.len(), &mut found) };
    // SAFETY: gr_name points to a NUL-terminated string in buf.
    (rc == 0 && !found.is_null()).then(|| unsafe { std::ffi::CStr::from_ptr(gr.gr_name) }.to_string_lossy().into_owned())
}

#[cfg(not(unix))]
fn group_name(_: u32) -> Option<String> {
    None
}

#[cfg(not(unix))]
fn chain(_: &Path) -> Result<Chain> {
    bail!("NixOS only")
}

/// Who may have the configuration rebuilt: its folder's owner (uid). Refused when another account
/// besides root and the owner can change the folder or one above it (a folder others can write
/// to with the sticky bit, such as /tmp, counts as changeable), or when `requester` is another
/// user than the owner or root. A folder its group can write to counts as unchangeable when the
/// group is one of `trusted` (`services.furnish.nix.trustedGroups`), whose members may become
/// root anyway; `group` names a gid. The setgid bit changes nothing.
pub fn may_rebuild(chain: &Chain, requester: u32, trusted: &[String], group: impl Fn(u32) -> Option<String>) -> Result<u32> {
    let Some((dir, owner, _, _)) = chain.first() else { bail!("no configuration folder") };
    for (d, uid, gid, mode) in chain {
        if *uid != 0 && uid != owner {
            bail!("{} belongs to another account than {}, which could change the configuration", d.display(), dir.display());
        }
        if mode & 0o002 != 0 {
            bail!("others can change {}, so furnish doesn't rebuild the configuration in {}", d.display(), dir.display());
        }
        if mode & 0o020 != 0 {
            let name = group(*gid);
            if !name.as_ref().is_some_and(|n| trusted.contains(n)) {
                let name = name.unwrap_or_else(|| gid.to_string());
                bail!(
                    "the group {name} can change {}, so furnish doesn't rebuild the configuration in {}. \
                     If every member of {name} may become root anyway, add it to services.furnish.nix.trustedGroups",
                    d.display(),
                    dir.display()
                );
            }
        }
    }
    if *owner != 0 && requester != *owner && requester != 0 {
        bail!("the NixOS configuration in {} belongs to another user: furnish rebuilds it only when that user asks", dir.display());
    }
    Ok(*owner)
}

/// Whether root may run git and Nix in a repository whose git folder belongs to another account
/// (`git`: path, owner, group, mode), as in a configuration the admins share (/etc/nixos as
/// root:wheel 2775, its git folder an admin's). Git and Nix's libgit2 refuse such a repository
/// unless `safe.directory` allows it, because its hooks and settings would run as root. furnish
/// allows it only when the owner is in a group of `trusted` (`services.furnish.nix.trustedGroups`),
/// whose members may become root anyway, and nobody else can change the git folder: not others,
/// and its group only if trusted. `member` says whether a uid is in a named group (`None` for the
/// owner: no such account).
pub fn may_trust_git(
    git: &(PathBuf, u32, u32, u32),
    trusted: &[String],
    group: impl Fn(u32) -> Option<String>,
    member: impl Fn(u32, &str) -> Option<bool>,
) -> Result<()> {
    let (path, uid, gid, mode) = git;
    if *uid != 0 {
        let groups = trusted.iter().map(|g| member(*uid, g)).collect::<Option<Vec<bool>>>();
        match groups {
            None => bail!("{} belongs to the id {uid}, which has no account, so furnish doesn't run git in it as root", path.display()),
            Some(g) if !g.contains(&true) => bail!(
                "{} belongs to {}, who isn't in a group in services.furnish.nix.trustedGroups, so furnish doesn't run git in it as root",
                path.display(),
                lookup(*uid).map_or_else(|| uid.to_string(), |a| a.name)
            ),
            Some(_) => {}
        }
    }
    if mode & 0o002 != 0 {
        bail!("others can change {}, so furnish doesn't run git in it as root", path.display());
    }
    if mode & 0o020 != 0 && !group(*gid).is_some_and(|n| trusted.contains(&n)) {
        let name = group(*gid).unwrap_or_else(|| gid.to_string());
        bail!("the group {name} can change {}, so furnish doesn't run git in it as root", path.display());
    }
    Ok(())
}

/// Whether an account is in a group: as its primary group or as a listed member.
#[cfg(unix)]
fn in_group(account: &Account, name: &str) -> bool {
    let Ok(cname) = std::ffi::CString::new(name) else { return false };
    let mut buf = vec![0 as libc::c_char; 64 * 1024];
    // SAFETY: zeroed group is a valid out-parameter; getgrnam_r fills it with pointers into buf.
    let mut gr: libc::group = unsafe { std::mem::zeroed() };
    let mut found: *mut libc::group = std::ptr::null_mut();
    let rc = unsafe { libc::getgrnam_r(cname.as_ptr(), &mut gr, buf.as_mut_ptr(), buf.len(), &mut found) };
    if rc != 0 || found.is_null() {
        return false;
    }
    if gr.gr_gid == account.gid {
        return true;
    }
    let mut p = gr.gr_mem;
    // SAFETY: gr_mem is a NULL-terminated array of NUL-terminated strings in buf.
    while !p.is_null() && !unsafe { *p }.is_null() {
        if unsafe { std::ffi::CStr::from_ptr(*p) }.to_bytes() == account.name.as_bytes() {
            return true;
        }
        p = unsafe { p.add(1) };
    }
    false
}

#[cfg(not(unix))]
fn in_group(_: &Account, _: &str) -> bool {
    false
}

/// A git setting file that lets root's git and Nix use a repository whose git folder belongs to
/// a trusted admin (`may_trust_git`), for the commands of one install. git and libgit2 read
/// `safe.directory` only from the user's and the system's settings, not from the repository or
/// (libgit2) from GIT_CONFIG_* variables, so this is root's git settings folder for these
/// commands: XDG_CONFIG_HOME points at a folder of furnish's whose git/config includes root's
/// own settings and allows just this repository. Its nix entry points at root's own Nix settings.
/// It also sets `core.sharedRepository = group` unless the repository says otherwise, so what
/// root's commit adds to the git folder stays changeable for the group.
struct GitTrust {
    dir: PathBuf,
}

impl GitTrust {
    /// `None` when root's git needs no help: the configuration isn't root's, it isn't in a git
    /// repository, or the git folder is root's.
    fn new(dir: &Path, owner: &Account, trusted: &[String]) -> Result<Option<GitTrust>> {
        if owner.uid != 0 {
            return Ok(None);
        }
        let Some(top) = dir.ancestors().find(|d| d.join(".git").exists()) else { return Ok(None) };
        let git = top.join(".git");
        #[cfg(unix)]
        let facts = {
            use std::os::unix::fs::MetadataExt;
            let m = std::fs::symlink_metadata(&git).with_context(|| format!("reading {}", git.display()))?;
            (git.clone(), m.uid(), m.gid(), m.mode())
        };
        #[cfg(not(unix))]
        let facts = (git.clone(), 0, 0, 0);
        if facts.1 == 0 {
            return Ok(None);
        }
        may_trust_git(&facts, trusted, group_name, |uid, g| lookup(uid).map(|a| in_group(&a, g)))?;

        let original = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| owner.home.join(".config"));
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
        let tmp = std::env::temp_dir().join(format!("furnish-git-{}-{nanos}", std::process::id()));
        // Fails if anything is there already, so nobody can have prepared it.
        #[cfg(unix)]
        let created = {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new().mode(0o700).create(&tmp)
        };
        #[cfg(not(unix))]
        let created = std::fs::create_dir(&tmp);
        created.with_context(|| format!("creating {}", tmp.display()))?;
        let trust = GitTrust { dir: tmp };
        std::fs::create_dir(trust.dir.join("git"))?;
        let mut tops = vec![top.to_path_buf()];
        if let Ok(real) = top.canonicalize()
            && real != top
        {
            tops.push(real);
        }
        std::fs::write(trust.dir.join("git/config"), git_config(&original.join("git/config"), &tops))?;
        #[cfg(unix)]
        if original.join("nix").exists() {
            std::os::unix::fs::symlink(original.join("nix"), trust.dir.join("nix"))?;
        }
        Ok(Some(trust))
    }

    fn apply(&self, c: &mut Command) {
        c.env("XDG_CONFIG_HOME", &self.dir);
    }
}

impl Drop for GitTrust {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The git settings of `GitTrust`: root's own (`include`), safe.directory for `tops`.
fn git_config(include: &Path, tops: &[PathBuf]) -> String {
    let quote = |p: &Path| format!("\"{}\"", p.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\""));
    let mut text = format!("[include]\n\tpath = {}\n[safe]\n", quote(include));
    for top in tops {
        text += &format!("\tdirectory = {}\n", quote(top));
    }
    text + "[core]\n\tsharedRepository = group\n"
}

/// Why furnish runs an app's steps itself instead of the furnish service, as `catalog` decides
/// it. The app's document carries it (`OUTSIDE_SERVICE_KEY` in `metadata.furnish`), and the
/// engine passes it to the resources as `OUTSIDE_SERVICE` (`pass_outside_service`), so the
/// refusal to change the NixOS configuration names the actual reason.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum OutsideService {
    /// The policy has no `service` section (on NixOS: `services.furnish.service.enable` is off).
    Off,
    /// The service installs only the apps of preset collections, and this collection isn't one.
    NotPreset { collection: String },
    /// The app installs for each person (`perUser`), so its steps run as the person.
    PerUser { app: String },
}

/// The variable with the `OutsideService` of the running operation, as JSON (empty: unknown).
pub const OUTSIDE_SERVICE: &str = "FURNISH_OUTSIDE_SERVICE";
/// Its key in the document's `metadata.furnish`.
pub const OUTSIDE_SERVICE_KEY: &str = "outsideService";

/// Sets `OUTSIDE_SERVICE` from a document before DSC runs it. The variable exists since
/// `engine::init`, so this only replaces its value in place, as `resource_env::refresh` does.
pub fn pass_outside_service(doc: &Value) {
    let value = match &doc["metadata"]["furnish"][OUTSIDE_SERVICE_KEY] {
        Value::Null => String::new(),
        v => v.to_string(),
    };
    if std::env::var(OUTSIDE_SERVICE).unwrap_or_default() != value {
        // SAFETY: the name exists since `engine::init` (see above); only the engine's running
        // operation changes it.
        unsafe { std::env::set_var(OUTSIDE_SERVICE, value) };
    }
}

/// Why furnish doesn't change the NixOS configuration outside the furnish service, and what the
/// administrator can do. The desktop app translates these texts (gui/src/lib/i18n.ts,
/// `knownError`): keep them in step.
pub fn service_only(why: Option<&OutsideService>) -> String {
    match why {
        Some(OutsideService::NotPreset { collection }) => format!(
            "only the furnish service changes the NixOS configuration, and it installs only the apps of preset collections. \
             The collection '{collection}' isn't preset: ask your administrator to preset it (services.furnish.policy.collections.preset) \
             or to list its furnish address in the policy (services.furnish.policy.endpoints), and, unless you're an administrator, \
             to set services.furnish.policy.service.allowedGroup to a group you're in"
        ),
        Some(OutsideService::PerUser { app }) => format!(
            "only the furnish service changes the NixOS configuration, and {app} installs for each person (perUser), so furnish runs its steps as you"
        ),
        Some(OutsideService::Off) | None => {
            "only the furnish service changes the NixOS configuration: ask your administrator to turn it on (services.furnish.service.enable)".into()
        }
    }
}

/// Prints a log line for furnish (DSC passes the resource's stderr lines on).
fn info(message: &str) {
    if crate::service::in_service() {
        crate::engine::emit_log("info", message.to_string());
    } else {
        eprintln!("{}", json!({ "info": message }));
    }
}

/// A command as the configuration's owner: their ids (no other groups), home and name.
#[cfg_attr(not(unix), allow(unused_variables))]
fn as_owner(owner: &Account, program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut c = Command::new(program);
    for var in crate::catalog::GIT_LOCATION_VARS {
        c.env_remove(var);
    }
    // Also for root, whose configuration the service runs these for (/etc/nixos): without HOME,
    // git wouldn't read root's settings, such as the name it commits flake.lock with.
    #[cfg(unix)]
    if is_root() {
        use std::os::unix::process::CommandExt;
        if owner.uid != 0 {
            c.uid(owner.uid).gid(owner.gid);
        }
        c.env("HOME", &owner.home).env("USER", &owner.name).env("LOGNAME", &owner.name);
        c.current_dir(&owner.home);
    }
    // Nix's programs, also for git (flakes) and Home Manager's activation.
    if let Some(bin) = nix_program(owner).parent().filter(|b| !b.as_os_str().is_empty()) {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut dirs = vec![bin.to_path_buf()];
        dirs.extend(std::env::split_paths(&path));
        if let Ok(joined) = std::env::join_paths(dirs) {
            c.env("PATH", joined);
        }
    }
    c.stdin(Stdio::null());
    c
}

/// The nix program: on PATH, else where NixOS and Nix's installers put it. A desktop app or the
/// furnish service may start without it on PATH, e.g. on macOS or another Linux.
fn nix_program(owner: &Account) -> PathBuf {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let on_path = std::env::split_paths(&path).map(|d| d.join("nix"));
    let usual = [
        PathBuf::from("/run/current-system/sw/bin/nix"),
        PathBuf::from("/nix/var/nix/profiles/default/bin/nix"),
        owner.home.join(".nix-profile/bin/nix"),
        owner.home.join(".local/state/nix/profile/bin/nix"),
        PathBuf::from(format!("/etc/profiles/per-user/{}/bin/nix", owner.name)),
    ];
    on_path.chain(usual).find(|p| p.is_file()).unwrap_or_else(|| PathBuf::from("nix"))
}

/// Runs a command; its output goes to the log, its error into the failure.
fn run(mut c: Command, what: &str) -> Result<String> {
    let out = c.stderr(Stdio::piped()).stdout(Stdio::piped()).output().with_context(|| format!("{what}: can't start it"))?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        let tail: Vec<&str> = stderr.lines().rev().take(15).collect();
        bail!("{what} failed:\n{}", tail.into_iter().rev().collect::<Vec<_>>().join("\n"));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn nix(owner: &Account) -> Command {
    let mut c = as_owner(owner, nix_program(owner));
    c.args(["--extra-experimental-features", "nix-command flakes"]);
    c
}

/// Reads a file of the owner's with the owner's rights (`None`: it doesn't exist).
fn owner_read(owner: &Account, path: &Path) -> Result<Option<Vec<u8>>> {
    let mut c = as_owner(owner, "cat");
    c.arg("--").arg(path);
    let out = c.stderr(Stdio::null()).output()?;
    Ok(out.status.success().then_some(out.stdout))
}

/// Writes a file of the owner's with the owner's rights.
fn owner_write(owner: &Account, path: &Path, bytes: &[u8]) -> Result<()> {
    let mut c = as_owner(owner, "sh");
    c.args(["-c", "cat > \"$1\"", "sh"]).arg(path).stdin(Stdio::piped());
    let mut child = c.spawn()?;
    child.stdin.take().context("no stdin")?.write_all(bytes)?;
    if !child.wait()?.success() {
        bail!("can't write {}", path.display());
    }
    Ok(())
}

/// Install (or remove, `desired.exist` false) an app in the system's apps.json (`Place::System`,
/// or the person's part for `Place::HomeNixos`) and rebuild the system.
fn apply(target: &Target, desired: &Desired, place: Place, person: &Account) -> Result<()> {
    if !in_service() {
        let why = std::env::var(OUTSIDE_SERVICE).ok().and_then(|v| serde_json::from_str(&v).ok());
        bail!("{}", service_only(why.as_ref()));
    }
    let entry = desired.entry(place);
    if desired.exist {
        check_in(&desired.name, &entry, target.allowlist(place, &person.name).as_deref(), place)?;
    }
    let requester: u32 = std::env::var("FURNISH_REQUESTER_UID").ok().and_then(|v| v.parse().ok()).unwrap_or(u32::MAX);
    let uid = may_rebuild(&chain(&target.dir)?, requester, &target.facts.nix.trusted_groups, group_name)?;
    let owner = lookup(uid).with_context(|| format!("no account with the id {uid}"))?;
    let trust = GitTrust::new(&target.dir, &owner, &target.facts.nix.trusted_groups)?;
    // git and Nix in the configuration's repository, as its owner.
    let trusting = |mut c: Command| {
        if let Some(t) = &trust {
            t.apply(&mut c);
        }
        c
    };
    let git = || trusting(as_owner(&owner, "git"));
    let nix = || trusting(nix(&owner));

    let apps_dir = Path::new(APPS_DIR);
    let before = read_apps(apps_dir)?;
    let mut after = before.clone();
    let list = match place {
        Place::HomeNixos => &mut after.homes.entry(person.name.clone()).or_default().apps,
        _ => &mut after.apps,
    };
    if desired.exist {
        list.insert(desired.name.clone(), entry);
    } else {
        list.remove(&desired.name);
    }
    after.homes.retain(|_, h| !h.apps.is_empty());
    let dir = &target.dir;
    let lock = dir.join("flake.lock");
    // Committed afterwards only if nothing else changed (and it is a git repository).
    let mut status = git();
    status.arg("-C").arg(dir).args(["status", "--porcelain"]);
    let clean = run(status, "git status").is_ok_and(|s| s.is_empty());
    let old_lock = owner_read(&owner, &lock)?;
    write_apps(apps_dir, &after)?;
    let undo = |why: anyhow::Error| -> anyhow::Error {
        let mut problems = Vec::new();
        if let Err(e) = write_apps(apps_dir, &before) {
            problems.push(format!("putting back {APPS_DIR}: {e:#}"));
        }
        if let Some(old) = &old_lock
            && let Err(e) = owner_write(&owner, &lock, old)
        {
            problems.push(format!("putting back {}: {e:#}", lock.display()));
        }
        if problems.is_empty() { why } else { why.context(problems.join("; ")) }
    };

    info(&format!("Updating furnish/apps in {} as {}", lock.display(), owner.name));
    let mut update = nix();
    update.args(["flake", "update", "furnish/apps", "--flake"]).arg(dir);
    run(update, "nix flake update furnish/apps").map_err(&undo)?;

    info(&format!("Building the system from {dir} as {}", owner.name, dir = dir.display()));
    let mut build = nix();
    build.args(["build", "--no-link", "--print-out-paths"]).arg(format!(
        "{}#nixosConfigurations.\"{}\".config.system.build.toplevel",
        dir.display(),
        target.attribute
    ));
    let system = run(build, "building the system").map_err(&undo)?;
    if !system.starts_with("/nix/store/") || system.contains(char::is_whitespace) {
        return Err(undo(anyhow::anyhow!("building the system gave no store path: {system}")));
    }

    let mode = target.facts.nix.rebuild.as_deref().unwrap_or("switch");
    let mode = if mode == "boot" { "boot" } else { "switch" };
    info(&format!("Switching to {system} ({mode})"));
    let mut profile = Command::new("nix-env");
    profile.args(["-p", "/nix/var/nix/profiles/system", "--set", &system]);
    run(profile, "setting the system profile")?;
    // Outside the furnish service, which the new system may restart; as nixos-rebuild does.
    let mut switch = Command::new("systemd-run");
    switch
        .args(["-E", "LOCALE_ARCHIVE", "-E", "NIXOS_INSTALL_BOOTLOADER", "--collect", "--no-ask-password", "--pipe", "--quiet"])
        .args(["--service-type=exec", "--unit=furnish-switch-to-configuration", "--wait"])
        .arg(format!("{system}/bin/switch-to-configuration"))
        .arg(mode);
    run(switch, "switch-to-configuration")?;
    commit(git(), dir, clean, desired);
    Ok(())
}

#[cfg(unix)]
fn is_root() -> bool {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() == 0 }
}

#[cfg(not(unix))]
fn is_root() -> bool {
    false
}

/// The state DSC sees: the app's entry in a local flake, and where.
fn state(name: &str, found: Option<&(Place, Entry)>) -> Value {
    match found {
        Some((Place::System, e)) => json!({ "name": name, "packages": e.packages, "options": e.options, "installedInto": "system", "_exist": true }),
        Some((p, e)) => json!({ "name": name, "packages": e.packages, "homeOptions": e.options, "installedInto": p.name(), "_exist": true }),
        None => json!({ "name": name, "_exist": false }),
    }
}

/// Arguments DSC passes to either the CLI or the desktop executable for Furnish/NixOS.
#[derive(clap::Parser)]
#[command(name = "nixos-resource")]
pub struct ResourceCommand {
    action: String,
    #[arg(long)]
    what_if: bool,
}

impl ResourceCommand {
    /// The desktop app has no general CLI parser. Recognize the same resource invocation
    /// before initializing GTK or the Tauri single-instance handler.
    pub fn from_desktop_args() -> Option<Self> {
        use clap::Parser as _;
        let mut args = std::env::args_os().skip(1);
        let command = args.next()?;
        (command == "nixos-resource").then(|| Self::parse_from(std::iter::once(command).chain(args)))
    }

    pub fn run(&self) -> ExitCode {
        resource(&self.action, self.what_if)
    }
}

/// `furnish nixos-resource get|test|set [--what-if]`: the resource Furnish/NixOS (its manifest
/// runs furnish-nixos.dsc.resource.sh, which runs this). Errors as furnish's sh resources do:
/// exit 11 for a missing part of the setup, a plain line on stderr otherwise.
fn resource(action: &str, what_if: bool) -> ExitCode {
    match resource_inner(action, what_if) {
        Ok(out) => {
            crate::cli_println!("{out}");
            ExitCode::SUCCESS
        }
        Err(e) if e.is::<Missing>() => {
            eprintln!("furnish: {e}");
            ExitCode::from(11)
        }
        Err(e) => {
            eprintln!("furnish: {e:#}");
            ExitCode::from(1)
        }
    }
}

fn resource_inner(action: &str, what_if: bool) -> Result<Value> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let desired: Desired = serde_json::from_str(&input).context("reading the resource's properties")?;
    anyhow::ensure!(explicit_name(&desired.name), "invalid Furnish/NixOS resource name");
    let person = person()?;
    let current = find(&desired.name, &person)?;
    operate(&desired, &person, current, action, what_if, &desired.name)
}

fn operate(desired: &Desired, person: &Account, current: Option<(Place, Entry)>, action: &str, what_if: bool, label: &str) -> Result<Value> {
    if current.is_none() && let Some((place, dir)) = external_options::probe(desired, person) {
        return external_options::response(desired, place, &dir, action, what_if);
    }
    let in_desired_state = match &current {
        Some((place, entry)) => desired.exist && *entry == desired.entry(*place),
        None => !desired.exist,
    };
    match action {
        "get" => Ok(state(&desired.name, current.as_ref())),
        "test" => {
            let mut s = state(&desired.name, current.as_ref());
            s["_inDesiredState"] = in_desired_state.into();
            Ok(s)
        }
        "set" if what_if => {
            // Installing: where it would go (or stays); removing: where it is.
            let place = match &current {
                Some((p, _)) if !desired.exist || in_desired_state => *p,
                _ if desired.exist => choose(desired, person)?,
                _ => Place::System,
            };
            let mut s = state(&desired.name, desired.exist.then(|| (place, desired.entry(place))).as_ref());
            let what = place.what();
            let step = match (in_desired_state, desired.exist) {
                (true, true) => format!("{label} is in {what}"),
                (true, false) => format!("{label} is not in {what}"),
                (false, true) => format!("Add {label} to {what} and rebuild it"),
                (false, false) => format!("Remove {label} from {what} and rebuild it"),
            };
            let mut lines = vec![json!(step)];
            if desired.exist && !in_desired_state {
                let allowlist = match place {
                    Place::HomeStandalone => HomeTarget::find(person)?.allowlist,
                    p => Target::here()?.allowlist(p, &person.name),
                };
                if let Err(e) = check_in(&desired.name, &desired.entry(place), allowlist.as_deref(), place) {
                    lines.push(json!(format!("refused: {e:#}")));
                }
            }
            s["_metadata"] = json!({ "whatIf": lines });
            Ok(s)
        }
        "set" => {
            if !in_desired_state {
                set(desired, person, current.as_ref().map(|(p, _)| *p))?;
            }
            Ok(state(&desired.name, find(&desired.name, person)?.as_ref()))
        }
        _ => bail!("usage: get|test|set [--what-if]"),
    }
}

/// The service stamps this on its verified, reconstructed package step. The only operation
/// permitted with it is the identical declarative change; no other backend may run.
pub(crate) const PACKAGE_FINGERPRINT: &str = "furnishNixPackageFingerprint";

/// Whether a package operation belongs to the configuration. Probe the native backend only
/// when the selected destination is ready, to respect an existing profile or external command.
pub(crate) fn package(
    props: &Value, attr: &str, action: &str, what_if: bool, probe: impl FnOnce() -> Result<Value>,
) -> Result<Option<Value>> {
    let metadata = &props["_metadata"][PACKAGE];
    let Some(key) = metadata["key"].as_str().filter(|s| package_key(s)) else { return Ok(None) };
    let person = person()?;
    let exist = props["_exist"].as_bool().unwrap_or(true);
    let prefix = key.rsplit_once(':').expect("validated key").0;
    let found = find_entry(&person, |apps| {
        if let Some(entry) = apps.get_key_value(key)
            && (exist || metadata["removal"] != true || entry.1.packages.as_slice() == [attr])
        {
            return Ok(Some(entry));
        }
        // An explicitly authored uninstall step can have a different DSC name. It may
        // remove only this app's unambiguous declaration of this exact attribute.
        if exist || metadata["removal"] != true {
            return Ok(None);
        }
        let mut entries = apps.iter().filter(|(k, e)| package_key(k) && k.rsplit_once(':').is_some_and(|(app, _)| app == prefix)
            && e.options.is_empty() && e.packages.len() == 1 && e.packages[0] == attr);
        let entry = entries.next();
        anyhow::ensure!(entries.next().is_none(), "multiple declarations of {attr} in this app; use the original step name for removal");
        Ok(entry)
    })?;
    let mut desired = Desired { name: key.into(), packages: vec![attr.into()], exist, ..Default::default() };
    let current = found.map(|(place, name, entry)| {
        desired.name = name;
        (place, entry)
    });
    if let Some((_, entry)) = &current {
        anyhow::ensure!(entry.options.is_empty() && entry.packages.len() == 1,
            "the existing configuration entry for {attr} does not match a Furnish package declaration");
    }
    let constrained = metadata["versioned"] == true || props.get("nixpkgs").is_some() || props.get("targetVersion").is_some();
    if constrained {
        if current.is_none() {
            return Ok(None);
        }
        if exist && action == "set" {
            bail!("{attr} is installed in the configuration; uninstall it before changing its Nix source or version requirements");
        }
    }
    if current.is_none() {
        match choose(&desired, &person) {
            Ok(_) => {}
            Err(e) if e.is::<Missing>() => return Ok(None),
            Err(e) => return Err(e),
        }
    }
    let expected = props["_metadata"][PACKAGE_FINGERPRINT].as_str();
    if let Some(expected) = expected {
        anyhow::ensure!(in_service() && fingerprint(&desired)? == expected, "the requested Nix package differs from the current preset app");
    }
    let observed = probe()?;
    if current.is_none() && observed["_exist"] == true {
        return Ok(None);
    }
    // When an alias changes, do not claim or replace a package already installed elsewhere.
    let changed_attr = current.as_ref().is_some_and(|(_, entry)| entry.packages != desired.packages);
    if changed_attr && exist && action == "set" && observed["_exist"] == true {
        bail!("{attr} is already installed separately; uninstall the app's previous declaration before changing its package");
    }
    let result = operate(&desired, &person, current, action, what_if, attr)?;
    let mut state = json!({"name":attr, "packageManager":"nix", "_exist":result["_exist"]});
    if result["_exist"] == true && !changed_attr && matches!(observed["managedBy"].as_str(), Some("nixos" | "home-manager")) && observed["version"].is_string() {
        state["version"] = observed["version"].clone();
    }
    for member in ["_inDesiredState", "_metadata"] {
        if let Some(v) = result.get(member) { state[member] = v.clone(); }
    }
    if constrained && exist && action == "test" {
        state["_inDesiredState"] = false.into();
    }
    Ok(Some(state))
}

/// A selector into a preset app. The service derives the properties itself. The fingerprint
/// prevents applying a different declaration if the checkout changed, or the app is held.
#[derive(Debug, Serialize, Deserialize)]
pub struct ServiceStep {
    pub app: String,
    pub values: serde_json::Map<String, Value>,
    pub step: String,
    pub uninstall: bool,
    pub fingerprint: String,
}

fn fingerprint(desired: &Desired) -> Result<String> {
    use sha2::{Digest, Sha256};
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    Ok(STANDARD.encode(Sha256::digest(serde_json::to_vec(desired)?)))
}

/// Run a step whose properties DSC evaluated from the service's own preset collection. The
/// requester uid comes from the authenticated connection, and all normal allowlists still apply.
pub(crate) fn apply_service_step(properties: &Value, expected: &str) -> Result<()> {
    let desired: Desired = serde_json::from_value(properties.clone())?;
    if !explicit_name(&desired.name) || !in_service() || fingerprint(&desired)? != expected {
        bail!("the requested NixOS step differs from the current preset app");
    }
    let person = person()?;
    let current = find(&desired.name, &person)?;
    operate(&desired, &person, current, "set", false, &desired.name).map(drop)
}

/// A changed app first leaves its previous place. Delegate the whole transition before any
/// local change if it involves a system rebuild, so refusal leaves a standalone install intact.
fn set(desired: &Desired, person: &Account, current: Option<Place>) -> Result<()> {
    let place = match current {
        Some(p) if !desired.exist => p,
        _ => choose(desired, person)?,
    };
    if !in_service() && (place != Place::HomeStandalone || current.is_some_and(|p| p != Place::HomeStandalone)) {
        let outside = std::env::var(OUTSIDE_SERVICE).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok());
        if let Some(mut route) = outside.filter(|v| v["reason"] == "perUser" && v["step"].is_string()) {
            route["fingerprint"] = fingerprint(desired)?.into();
            let step = serde_json::from_value(route)?;
            crate::engine::capture_logs(|line| eprintln!("{}", json!({(line.level): line.message})));
            return crate::service::run(&crate::service::Request::NixosStep { step, install_into: own_install_into() }, |_| {});
        }
        let why = std::env::var(OUTSIDE_SERVICE).ok().and_then(|v| serde_json::from_str(&v).ok());
        bail!("{}", service_only(why.as_ref()));
    }
    if let Some(was) = current && was != place {
        put(was, &Desired { exist: false, ..desired.clone() }, person)?;
    }
    put(place, desired, person)
}

/// Installs or removes an app in one place.
fn put(place: Place, desired: &Desired, person: &Account) -> Result<()> {
    match place {
        Place::HomeStandalone => apply_home(&HomeTarget::find(person)?, desired, person),
        p => apply(&Target::here()?, desired, p, person),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_keys_follow_the_app_and_step_across_revisions() {
        let raw = json!({
            "resources": [
                {"type":"Furnish/Package", "name":"first", "properties":{"name":"a"}, "metadata": {
                    (PACKAGE): {"key":"another-app"}, (PACKAGE_FINGERPRINT):"forged"
                }},
                {"type":"Furnish/Package", "name":"second", "properties":{"name":"a", "versions":[">=1"]}}
            ],
            "metadata":{"furnish":{"uninstall":[{"type":"Furnish/Package","name":"first","properties":{"name":"a","_exist":false}}]}}
        });
        let stamp = |mut doc: Value, app| {
            stamp_packages(&mut doc, app).unwrap();
            doc
        };
        let a = stamp(raw.clone(), "org/one");
        let b = stamp(raw.clone(), "org/two");
        let key = &a["resources"][0]["metadata"][PACKAGE]["key"];
        assert!(package_key(key.as_str().unwrap()));
        assert!(!explicit_name(key.as_str().unwrap()));
        assert!(a["resources"][0]["metadata"].get(PACKAGE_FINGERPRINT).is_none());
        assert_ne!(key, &b["resources"][0]["metadata"][PACKAGE]["key"]);
        assert_ne!(key, &a["resources"][1]["metadata"][PACKAGE]["key"]);
        let removal = &a["metadata"]["furnish"]["uninstall"][0]["metadata"][PACKAGE];
        assert_eq!(key, &removal["key"]);
        assert_eq!(removal["removal"], true);
        assert_eq!(a["resources"][1]["metadata"][PACKAGE]["versioned"], true);
        let mut changed = raw;
        changed["resources"][0]["properties"]["name"] = "namespace.new".into();
        assert_eq!(key, &stamp(changed, "org/one")["resources"][0]["metadata"][PACKAGE]["key"]);
    }

    #[test]
    fn allowed_options_match_part_by_part() {
        assert!(allowed("programs.htop.enable", "programs.htop.enable"));
        assert!(allowed("programs.*.enable", "programs.htop.enable"));
        assert!(!allowed("programs.*.enable", "programs.htop.settings"));
        assert!(!allowed("programs.*", "programs.htop.enable"));
        assert!(!allowed("programs.htop.enable", "programs.htop.enable.x"));
        assert!(!allowed("programs.htop", "programs.htopx"));
    }

    #[test]
    fn entries_are_checked_against_the_allowlist() {
        let allow = vec!["programs.*.enable".to_string()];
        let mut e = Entry { packages: vec!["htop".into(), "kdePackages.kdenlive".into(), "gtk+3".into()], ..Default::default() };
        e.options.insert("programs.htop.enable".into(), json!(true));
        check("htop", &e, &allow).unwrap();
        e.options.insert("services.openssh.enable".into(), json!(true));
        for place in [Place::System, Place::HomeStandalone, Place::HomeNixos] {
            check_in("htop", &e, None, place).unwrap();
            assert!(check_in("htop", &e, Some(&[]), place).is_err());
        }
        let err = check("htop", &e, &allow).unwrap_err().to_string();
        assert!(err.contains("services.openssh.enable") && err.contains("allowedOptions"), "{err}");
        assert!(check("htop", &Entry { packages: vec!["a b".into()], ..Default::default() }, &allow).is_err());
        assert!(check("htop", &Entry { packages: vec!["${x}".into()], ..Default::default() }, &allow).is_err());
        assert!(check("a.b", &Entry::default(), &allow).is_err());
        let mut odd = Entry::default();
        odd.options.insert("programs.\"x\".enable".into(), json!(true));
        assert!(check_in("x", &odd, None, Place::System).is_err());
    }

    #[test]
    fn a_configuration_is_rebuilt_only_for_its_owner_or_root() {
        let none: &[String] = &[];
        let rebuild = |chain: &Chain, requester| may_rebuild(chain, requester, none, |_| Some("users".into()));
        let alice = |mode| -> Chain {
            vec![
                ("/home/alice/cfg".into(), 1000, 100, mode),
                ("/home/alice".into(), 1000, 100, 0o700),
                ("/home".into(), 0, 0, 0o755),
                ("/".into(), 0, 0, 0o755),
            ]
        };
        assert_eq!(rebuild(&alice(0o755), 1000).unwrap(), 1000);
        assert_eq!(rebuild(&alice(0o755), 0).unwrap(), 1000);
        let err = rebuild(&alice(0o755), 1001).unwrap_err().to_string();
        assert!(err.contains("another user"), "{err}");
        assert!(rebuild(&alice(0o775), 1000).is_err(), "the group can change it");
        let root: Chain = vec![("/etc/nixos".into(), 0, 0, 0o755), ("/etc".into(), 0, 0, 0o755), ("/".into(), 0, 0, 0o755)];
        assert_eq!(rebuild(&root, 1001).unwrap(), 0);
        let under_bob: Chain = vec![("/home/bob/x/cfg".into(), 1000, 100, 0o755), ("/home/bob/x".into(), 1001, 100, 0o755), ("/".into(), 0, 0, 0o755)];
        assert!(rebuild(&under_bob, 1000).is_err(), "bob could swap the folder");
        let tmp: Chain = vec![("/tmp/cfg".into(), 1000, 100, 0o755), ("/tmp".into(), 0, 0, 0o1777), ("/".into(), 0, 0, 0o755)];
        assert!(rebuild(&tmp, 1000).is_err());
    }

    #[test]
    fn a_folder_a_trusted_group_can_change_is_rebuilt() {
        // gid 1: wheel, 100: users, 7: a group without a name.
        let name = |gid: u32| match gid {
            0 => Some("root".to_string()),
            1 => Some("wheel".to_string()),
            100 => Some("users".to_string()),
            _ => None,
        };
        let wheel = ["wheel".to_string()];
        let etc_nixos = |gid, mode| -> Chain { vec![("/etc/nixos".into(), 0, gid, mode), ("/etc".into(), 0, 0, 0o755), ("/".into(), 0, 0, 0o755)] };
        // The shared /etc/nixos of #21: root:wheel 2775.
        assert_eq!(may_rebuild(&etc_nixos(1, 0o2775), 1000, &wheel, name).unwrap(), 0);
        assert_eq!(may_rebuild(&etc_nixos(1, 0o775), 1000, &wheel, name).unwrap(), 0);
        assert_eq!(may_rebuild(&etc_nixos(1, 0o2755), 1000, &[], name).unwrap(), 0, "setgid alone");
        let err = may_rebuild(&etc_nixos(1, 0o2775), 1000, &[], name).unwrap_err().to_string();
        assert!(err.contains("the group wheel can change /etc/nixos") && err.contains("services.furnish.nix.trustedGroups"), "{err}");
        let err = may_rebuild(&etc_nixos(100, 0o2775), 1000, &wheel, name).unwrap_err().to_string();
        assert!(err.contains("the group users can change /etc/nixos"), "another group: {err}");
        let err = may_rebuild(&etc_nixos(7, 0o775), 1000, &wheel, name).unwrap_err().to_string();
        assert!(err.contains("the group 7 can change"), "a group without a name: {err}");
        for mode in [0o2777, 0o777, 0o757, 0o3777] {
            let err = may_rebuild(&etc_nixos(1, mode), 1000, &wheel, name).unwrap_err().to_string();
            assert!(err.contains("others can change /etc/nixos"), "{mode:o}: {err}");
        }
        // A folder above it counts the same.
        let above = |gid| -> Chain { vec![("/srv/cfg/nixos".into(), 0, 0, 0o755), ("/srv/cfg".into(), 0, gid, 0o2775), ("/".into(), 0, 0, 0o755)] };
        assert_eq!(may_rebuild(&above(1), 1000, &wheel, name).unwrap(), 0);
        assert!(may_rebuild(&above(100), 1000, &wheel, name).unwrap_err().to_string().contains("the group users can change /srv/cfg"));
        // A person's configuration: a trusted group changes nothing about who may ask.
        let alice: Chain = vec![("/home/alice/cfg".into(), 1000, 1, 0o2775), ("/home/alice".into(), 1000, 100, 0o700), ("/".into(), 0, 0, 0o755)];
        assert_eq!(may_rebuild(&alice, 1000, &wheel, name).unwrap(), 1000);
        assert!(may_rebuild(&alice, 1001, &wheel, name).unwrap_err().to_string().contains("another user"));
    }

    #[test]
    fn root_runs_git_only_in_a_git_folder_of_a_trusted_admin() {
        // gid 1: wheel, 100: users; uid 1000 (alice) is in wheel, 1001 (bob) isn't, 1002 has no account.
        let name = |gid: u32| match gid {
            1 => Some("wheel".to_string()),
            100 => Some("users".to_string()),
            _ => None,
        };
        let member = |uid: u32, group: &str| match uid {
            0 | 1000 => Some(true),
            1001 => Some(group == "users"),
            _ => None,
        };
        let wheel = ["wheel".to_string()];
        let git = |uid, gid, mode| (PathBuf::from("/etc/nixos/.git"), uid, gid, mode);
        // The shared /etc/nixos of #24: the git folder alice:wheel, the group can change it.
        may_trust_git(&git(1000, 1, 0o2775), &wheel, name, member).unwrap();
        may_trust_git(&git(1000, 100, 0o755), &wheel, name, member).unwrap();
        let err = may_trust_git(&git(1001, 1, 0o2775), &wheel, name, member).unwrap_err().to_string();
        assert!(err.contains("/etc/nixos/.git belongs to") && err.contains("trustedGroups"), "{err}");
        let err = may_trust_git(&git(1002, 1, 0o755), &wheel, name, member).unwrap_err().to_string();
        assert!(err.contains("has no account"), "{err}");
        let err = may_trust_git(&git(1000, 1, 0o2775), &[], name, member).unwrap_err().to_string();
        assert!(err.contains("trustedGroups"), "no trusted group: {err}");
        let err = may_trust_git(&git(1000, 100, 0o2775), &wheel, name, member).unwrap_err().to_string();
        assert!(err.contains("the group users can change /etc/nixos/.git"), "{err}");
        let err = may_trust_git(&git(1000, 1, 0o777), &wheel, name, member).unwrap_err().to_string();
        assert!(err.contains("others can change"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn an_account_is_in_its_primary_group() {
        let me = lookup(current_uid()).unwrap();
        assert!(in_group(&me, &group_name(me.gid).unwrap()));
        assert!(!in_group(&me, "furnish-no-such-group"));
    }

    #[test]
    fn git_reads_the_trust_settings_with_the_own_ones() {
        let tmp = tempfile::tempdir().unwrap();
        let own = tmp.path().join("own");
        std::fs::create_dir_all(own.join("git")).unwrap();
        std::fs::write(own.join("git/config"), "[user]\n\tname = Root\n").unwrap();
        let trust = tmp.path().join("trust");
        std::fs::create_dir_all(trust.join("git")).unwrap();
        let tops = [PathBuf::from("/etc/nixos"), PathBuf::from("/srv/a \"b\"")];
        std::fs::write(trust.join("git/config"), git_config(&own.join("git/config"), &tops)).unwrap();
        let get = |args: &[&str]| {
            let out = Command::new("git")
                .args(args)
                .env("XDG_CONFIG_HOME", &trust)
                .env("HOME", tmp.path())
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .current_dir(tmp.path())
                .output()
                .unwrap();
            String::from_utf8(out.stdout).unwrap()
        };
        assert_eq!(get(&["config", "--global", "--get-all", "safe.directory"]), "/etc/nixos\n/srv/a \"b\"\n");
        assert_eq!(get(&["config", "--global", "--includes", "user.name"]), "Root\n");
        assert_eq!(get(&["config", "--global", "core.sharedRepository"]), "group\n");
    }

    #[test]
    fn the_refusal_outside_the_service_names_the_reason() {
        // gui/src/lib/i18n.test.ts translates these texts: keep both in step.
        let off = service_only(Some(&OutsideService::Off));
        assert_eq!(
            off,
            "only the furnish service changes the NixOS configuration: ask your administrator to turn it on (services.furnish.service.enable)"
        );
        assert_eq!(service_only(None), off, "unknown: resources run by DSC itself");
        let mine = OutsideService::NotPreset { collection: "mine".into() };
        assert_eq!(
            service_only(Some(&mine)),
            "only the furnish service changes the NixOS configuration, and it installs only the apps of preset collections. \
             The collection 'mine' isn't preset: ask your administrator to preset it (services.furnish.policy.collections.preset) \
             or to list its furnish address in the policy (services.furnish.policy.endpoints), and, unless you're an administrator, \
             to set services.furnish.policy.service.allowedGroup to a group you're in"
        );
        assert_eq!(
            service_only(Some(&OutsideService::PerUser { app: "acme-ssh".into() })),
            "only the furnish service changes the NixOS configuration, and acme-ssh installs for each person (perUser), so furnish runs its steps as you"
        );
        // As the engine passes it (OUTSIDE_SERVICE) and the document carries it.
        let json = serde_json::to_string(&mine).unwrap();
        assert_eq!(json, r#"{"reason":"notPreset","collection":"mine"}"#);
        assert_eq!(serde_json::from_str::<OutsideService>(&json).unwrap(), mine);
        assert_eq!(serde_json::to_string(&OutsideService::Off).unwrap(), r#"{"reason":"off"}"#);
    }

    #[test]
    fn accounts_come_from_passwd() {
        let passwd = "root:x:0:0:System administrator:/root:/bin/sh\nalice:x:1000:100::/home/alice:/bin/sh\n";
        assert_eq!(account(passwd, 1000), Some(Account { uid: 1000, gid: 100, name: "alice".into(), home: "/home/alice".into() }));
        assert_eq!(account(passwd, 5), None);
    }

    #[test]
    fn apps_go_home_when_the_module_is_set_up_unless_the_setting_or_their_options_say_otherwise() {
        let leaf = |v: &str| crate::settings::Leaf { value: json!(v), set_by: crate::settings::SetBy::User, locked: false };
        assert_eq!(install_into(None, false), "system");
        assert_eq!(install_into(None, true), "home");
        assert_eq!(install_into(Some(&leaf("system")), true), "system");
        assert_eq!(install_into(Some(&leaf("home")), false), "home");
        let app = |options: &[&str], home: &[&str]| Desired {
            name: "a".into(),
            options: options.iter().map(|o| (o.to_string(), json!(true))).collect(),
            home_options: home.iter().map(|o| (o.to_string(), json!(true))).collect(),
            exist: true,
            ..Default::default()
        };
        let h = Place::HomeStandalone;
        assert_eq!(pick(h, h, &app(&[], &[])), h, "packages only");
        assert_eq!(pick(h, h, &app(&["programs.x.enable"], &["programs.x.enable"])), h);
        assert_eq!(pick(h, h, &app(&["services.x.enable"], &[])), Place::System, "NixOS options only");
        assert_eq!(pick(Place::System, Place::HomeNixos, &app(&[], &["programs.x.enable"])), Place::HomeNixos, "Home Manager options only");
        assert_eq!(pick(Place::System, h, &app(&["services.x.enable"], &[])), Place::System);
        let d = app(&["services.x.enable"], &["programs.x.enable"]);
        assert_eq!(d.entry(Place::System).options.keys().collect::<Vec<_>>(), ["services.x.enable"]);
        assert_eq!(d.entry(h).options.keys().collect::<Vec<_>>(), ["programs.x.enable"]);
    }

    #[test]
    fn a_step_only_where_set_up_is_skipped_elsewhere() {
        // The owner's example (#8): Home Manager writes the Include, Furnish/Line finds it.
        let ssh = json!({"name": "SSH include through Home Manager", "type": "Furnish/NixOS",
            "properties": {"name": "acme-ssh", "onlyIfSetUp": true, "homeOptions": {"programs.ssh.includes": ["config.d/acme.conf"]}}});
        let ready = |_: &Desired| None;
        assert_eq!(not_needed(&ssh, false, ready), Some(NO_NIX), "Windows, or no Nix");
        assert_eq!(not_needed(&ssh, true, |_| Some(Place::HomeStandalone)), Some(NO_HOME));
        assert_eq!(not_needed(&ssh, true, |_| Some(Place::HomeNixos)), Some(NO_HOME));
        assert_eq!(not_needed(&ssh, true, |_| Some(Place::System)), Some(NO_SYSTEM));
        assert_eq!(not_needed(&ssh, true, ready), None, "set up: the step applies");
        let mut plain = ssh.clone();
        plain["properties"].as_object_mut().unwrap().remove("onlyIfSetUp");
        assert_eq!(not_needed(&plain, false, ready), None, "without the property, nothing changes");
        plain["properties"]["onlyIfSetUp"] = json!(false);
        assert_eq!(not_needed(&plain, false, ready), None);
        let line = json!({"name": "SSH include", "type": "Furnish/Line", "properties": {"onlyIfSetUp": true}});
        assert_eq!(not_needed(&line, false, ready), None, "only Furnish/NixOS has it");
        // The resource reads its properties with it.
        let d: Desired = serde_json::from_value(ssh["properties"].clone()).unwrap();
        assert_eq!(d.entry(Place::HomeStandalone).options["programs.ssh.includes"], json!(["config.d/acme.conf"]));
    }

    #[test]
    fn the_home_setup_line_names_the_folder() {
        let line = home_setup_line(Path::new("/home/alice/.local/share/furnish/home-apps"));
        assert_eq!(line, "inputs.furnish.inputs.home-apps.url = \"path:/home/alice/.local/share/furnish/home-apps\";");
        assert_eq!(line_url(&line), "path:/home/alice/.local/share/furnish/home-apps");
        assert!(home_apps_dir().ends_with("furnish/home-apps"));
        // The configuration says where its input points; flake.lock first.
        let flake = r#"{ inputs.furnish.inputs.home-apps.url = "path:/home/alice/data/furnish/home-apps"; }"#;
        assert_eq!(configured_home_apps(None, flake), Some(PathBuf::from("/home/alice/data/furnish/home-apps")));
        let nested = r#"furnish = { url = "github:x/y"; inputs.home-apps.url = "path:/srv/a"; };"#;
        assert_eq!(configured_home_apps(None, nested), Some(PathBuf::from("/srv/a")));
        assert_eq!(configured_home_apps(None, "{ }"), None);
        assert_eq!(configured_home_apps(None, r#"home-apps.url = "path:./relative";"#), None);
        let lock = r#"{"root": "root", "version": 7, "nodes": {
            "root": {"inputs": {"furnish": "furnish", "nixpkgs": "nixpkgs"}},
            "furnish": {"inputs": {"home-apps": "home-apps_2", "nixpkgs": ["nixpkgs"]}},
            "home-apps_2": {"locked": {"type": "path", "path": "/home/alice/locked"}, "original": {"type": "path", "path": "/home/alice/locked"}}}}"#;
        assert_eq!(configured_home_apps(Some(lock), flake), Some(PathBuf::from("/home/alice/locked")));
        assert_eq!(configured_home_apps(Some("{}"), flake), Some(PathBuf::from("/home/alice/data/furnish/home-apps")));
    }

    #[test]
    fn homes_are_kept_apart_and_left_out_when_empty() {
        let mut apps = Apps::default();
        assert!(!serde_json::to_string(&apps).unwrap().contains("homes"));
        apps.homes.entry("alice".into()).or_default().apps.insert("hello".into(), Entry { packages: vec!["hello".into()], ..Default::default() });
        let text = serde_json::to_string(&apps).unwrap();
        assert_eq!(serde_json::from_str::<Apps>(&text).unwrap(), apps);
        assert!(text.contains(r#""homes":{"alice":{"apps":{"hello""#), "{text}");
    }

    #[test]
    fn the_local_flake_round_trips() {
        let dir = std::env::temp_dir().join(format!("furnish-apps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(read_apps(&dir).unwrap(), Apps::default());
        let mut apps = Apps::default();
        let mut e = Entry { packages: vec!["hello".into()], ..Default::default() };
        e.options.insert("programs.htop.enable".into(), json!(true));
        apps.apps.insert("htop".into(), e);
        write_apps(&dir, &apps).unwrap();
        assert_eq!(read_apps(&dir).unwrap(), apps);
        assert_eq!(std::fs::read_to_string(dir.join("flake.nix")).unwrap(), APPS_FLAKE);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
