//! Furnish/NixOS: apps that go into the computer's NixOS configuration, a flake, instead of a
//! profile. furnish never edits the configuration's files. Its flake has an input `apps`, which
//! the configuration points at furnish's local flake once, by hand:
//!
//! ```nix
//! inputs.furnish.inputs.apps.url = "path:/var/lib/furnish/apps";
//! ```
//!
//! furnish's NixOS module adds what that flake lists (`apps.json`): packages from the system's
//! nixpkgs, and NixOS options from `services.furnish.nix.allowedOptions` (`allowed`).
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
//! never if anyone else can change it or a folder above it. Steps 2, 3 and 4 run as the folder's
//! owner, not as root: then no root process opens a git repository another account controls
//! (git's hooks and config would run as root; libgit2 in Nix refuses such a repository), and
//! furnish reads and writes the owner's files only with the owner's rights. Only the switch runs
//! as root.

use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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

/// furnish's local flake for a person's standalone Home Manager configuration: `furnish/home-apps`
/// in their data folder (`$XDG_DATA_HOME`, by default `~/.local/share`). `home`: someone else's
/// home folder (in the service), where furnish can't see their environment.
pub fn home_apps_dir(home: Option<&Path>) -> PathBuf {
    let data = match home {
        Some(h) => h.join(".local/share"),
        None => dirs::data_dir().unwrap_or_else(|| PathBuf::from(".local/share")),
    };
    data.join("furnish/home-apps")
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

/// Checks an app's entry: package and option names, and each option against the allowlist.
pub fn check(name: &str, entry: &Entry, allowlist: &[String]) -> Result<()> {
    check_in(name, entry, allowlist, Place::System)
}

fn check_in(name: &str, entry: &Entry, allowlist: &[String], place: Place) -> Result<()> {
    if !plain_name(name, "") || name.contains('.') {
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
        if !allowlist.iter().any(|a| allowed(a, o)) {
            match place {
                Place::System => bail!("the app sets the NixOS option {o}, which isn't in services.furnish.nix.allowedOptions on this computer"),
                _ => bail!("the app sets the Home Manager option {o}, which isn't in services.furnish.nix.allowedOptions in your Home Manager configuration"),
            }
        }
    }
    Ok(())
}

/// What the resource gets and returns (DSC's JSON on stdin and stdout).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Desired {
    name: String,
    #[serde(default)]
    packages: Vec<String>,
    #[serde(default)]
    options: BTreeMap<String, Value>,
    #[serde(default)]
    home_options: BTreeMap<String, Value>,
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
    allowed_options: Vec<String>,
    rebuild: Option<String>,
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
    /// The options apps may set there: the NixOS module's, or the person's from Home Manager's
    /// NixOS module, as the NixOS configuration says.
    fn allowlist(&self, place: Place, user: &str) -> Vec<String> {
        match place {
            Place::HomeNixos => self.facts.home_manager_users.get(user).map(|u| u.allowed_options.clone()).unwrap_or_default(),
            _ => self.facts.nix.allowed_options.clone(),
        }
    }
}

/// A person's standalone Home Manager configuration, ready for furnish to install into.
struct HomeTarget {
    dir: PathBuf,
    attribute: String,
    allowlist: Vec<String>,
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

/// The person's `osSpecific.nixos.installInto`, or the policy's. The service reads the person's
/// own settings for this one: it only chooses between places they may install into anyway.
fn install_into_for(person: &Account, module: bool) -> &'static str {
    let choices = if in_service() {
        read_as(person, &person.home.join(".local/share/furnish/settings.json")).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    } else {
        crate::settings::user_choices()
    };
    install_into(crate::nixos::chosen(&crate::policy::load(), &choices).install_into.as_ref(), module)
}

impl HomeTarget {
    fn find(person: &Account) -> Result<HomeTarget> {
        let Some(facts) = home_facts(person) else {
            return Err(missing("furnish's Home Manager module (services.furnish.enable in your Home Manager configuration)"));
        };
        let dir = match &facts.nix.configuration {
            Some(c) => PathBuf::from(c),
            None if !in_service() => crate::nixos::setup()
                .and_then(|s| s.home_manager_configuration)
                .map_or_else(|| person.home.join(".config/home-manager"), |f| f.path),
            None => person.home.join(".config/home-manager"),
        };
        let apps_dir = home_apps_dir(in_service().then_some(person.home.as_path()));
        let line = home_setup_line(&apps_dir);
        let Some(flake) = read_as(person, &dir.join("flake.nix")) else {
            return Err(missing(format!("a Home Manager configuration that is a flake ({} has no flake.nix)", dir.display())));
        };
        if !flake.contains(line_url(&line)) {
            return Err(missing(format!("the line {line} in {}", dir.join("flake.nix").display())));
        }
        let attribute = facts.nix.flake_attribute.clone().unwrap_or_else(|| person.name.clone());
        Ok(HomeTarget { dir, attribute, allowlist: facts.nix.allowed_options, apps_dir })
    }
}

/// The person furnish acts for: in the service, who asked; else who runs furnish.
fn person() -> Result<Account> {
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
    let uid = if in_service() {
        let requester = std::env::var("FURNISH_REQUESTER_UID").ok().and_then(|v| v.parse().ok());
        requester.context("the furnish service doesn't know who asked")?
    } else {
        current_uid()
    };
    Ok(account(&passwd, uid).unwrap_or_else(|| Account {
        uid,
        gid: uid,
        name: std::env::var("USER").unwrap_or_default(),
        home: dirs::home_dir().unwrap_or_default(),
    }))
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
    let system = read_apps(Path::new(APPS_DIR))?;
    if let Some(e) = system.apps.get(name) {
        return Ok(Some((Place::System, e.clone())));
    }
    if let Some(e) = system.homes.get(&person.name).and_then(|h| h.apps.get(name)) {
        return Ok(Some((Place::HomeNixos, e.clone())));
    }
    let own = read_apps_as(person, &home_apps_dir(in_service().then_some(person.home.as_path())))?;
    Ok(own.apps.get(name).map(|e| (Place::HomeStandalone, e.clone())))
}

/// Whether furnish can install into a place: `Missing` says what is missing.
fn ready(place: Place, person: &Account) -> Result<()> {
    match place {
        Place::System => Target::here().map(drop),
        Place::HomeNixos => {
            let target = Target::here()?;
            if !target.facts.home_manager_users.get(&person.name).is_some_and(|u| u.enable) {
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
    let facts = home_facts(person);
    let home = if facts.as_ref().is_some_and(|f| f.nixos_module) { Place::HomeNixos } else { Place::HomeStandalone };
    let wanted = if install_into_for(person, facts.is_some()) == "home" { home } else { Place::System };
    let place = pick(wanted, home, desired);
    ready(place, person)?;
    Ok(place)
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

/// Install (or remove) an app in the person's standalone Home Manager configuration: as the
/// person, update furnish/home-apps, build the generation and activate it.
fn apply_home(home: &HomeTarget, desired: &Desired, person: &Account) -> Result<()> {
    let entry = desired.entry(Place::HomeStandalone);
    if desired.exist {
        check_in(&desired.name, &entry, &home.allowlist, Place::HomeStandalone)?;
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
    run(as_owner(person, &format!("{generation}/activate")), "activating the Home Manager generation")?;
    commit(person, dir, clean, desired);
    Ok(())
}

/// Commits flake.lock as the configuration's owner if the repository had no other changes.
fn commit(owner: &Account, dir: &Path, clean: bool, desired: &Desired) {
    if clean {
        let verb = if desired.exist { "install" } else { "uninstall" };
        let mut commit = as_owner(owner, "git");
        commit.arg("-C").arg(dir).args(["commit", "-q", "-m", &format!("furnish: {verb} {}", desired.name), "--", "flake.lock"]);
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

/// The owner and mode of a folder and of each folder above it.
pub type Chain = Vec<(PathBuf, u32, u32)>;

#[cfg(unix)]
fn chain(dir: &Path) -> Result<Chain> {
    use std::os::unix::fs::MetadataExt;
    dir.ancestors()
        .map(|d| {
            let m = std::fs::symlink_metadata(d).with_context(|| format!("reading {}", d.display()))?;
            Ok((d.to_path_buf(), m.uid(), m.mode()))
        })
        .collect()
}

#[cfg(not(unix))]
fn chain(_: &Path) -> Result<Chain> {
    bail!("NixOS only")
}

/// Who may have the configuration rebuilt: its folder's owner (uid). Refused when another account
/// besides root and the owner can change the folder or one above it (a folder others can write
/// to with the sticky bit, such as /tmp, counts as changeable), or when `requester` is another
/// user than the owner or root.
pub fn may_rebuild(chain: &Chain, requester: u32) -> Result<u32> {
    let Some((dir, owner, _)) = chain.first() else { bail!("no configuration folder") };
    for (d, uid, mode) in chain {
        if *uid != 0 && uid != owner {
            bail!("{} belongs to another account than {}, which could change the configuration", d.display(), dir.display());
        }
        if mode & 0o022 != 0 {
            bail!("others can change {}, so furnish doesn't rebuild the configuration in {}", d.display(), dir.display());
        }
    }
    if *owner != 0 && requester != *owner && requester != 0 {
        bail!("the NixOS configuration in {} belongs to another user: furnish rebuilds it only when that user asks", dir.display());
    }
    Ok(*owner)
}

/// Prints a log line for furnish (DSC passes the resource's stderr lines on).
fn info(message: &str) {
    eprintln!("{}", json!({ "info": message }));
}

/// A command as the configuration's owner: their ids (no other groups), home and name.
#[cfg_attr(not(unix), allow(unused_variables))]
fn as_owner(owner: &Account, program: &str) -> Command {
    let mut c = Command::new(program);
    #[cfg(unix)]
    if owner.uid != 0 && is_root() {
        use std::os::unix::process::CommandExt;
        c.uid(owner.uid).gid(owner.gid);
        c.env("HOME", &owner.home).env("USER", &owner.name).env("LOGNAME", &owner.name);
        c.current_dir(&owner.home);
    }
    c.stdin(Stdio::null());
    c
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
    let mut c = as_owner(owner, "nix");
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
        bail!("only the furnish service changes the NixOS configuration: ask your administrator to turn it on (services.furnish.service.enable)");
    }
    let entry = desired.entry(place);
    if desired.exist {
        check_in(&desired.name, &entry, &target.allowlist(place, &person.name), place)?;
    }
    let requester: u32 = std::env::var("FURNISH_REQUESTER_UID").ok().and_then(|v| v.parse().ok()).unwrap_or(u32::MAX);
    let uid = may_rebuild(&chain(&target.dir)?, requester)?;
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
    let owner = account(&passwd, uid).with_context(|| format!("no account with the id {uid}"))?;

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
    let mut status = as_owner(&owner, "git");
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
    let mut update = nix(&owner);
    update.args(["flake", "update", "furnish/apps", "--flake"]).arg(dir);
    run(update, "nix flake update furnish/apps").map_err(&undo)?;

    info(&format!("Building the system from {dir} as {}", owner.name, dir = dir.display()));
    let mut build = nix(&owner);
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
    commit(&owner, dir, clean, desired);
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

/// `furnish nixos-resource get|test|set [--what-if]`: the resource Furnish/NixOS (its manifest
/// runs furnish-nixos.dsc.resource.sh, which runs this). Errors as furnish's sh resources do:
/// exit 11 for a missing part of the setup, a plain line on stderr otherwise.
pub fn resource(action: &str, what_if: bool) -> ExitCode {
    match resource_inner(action, what_if) {
        Ok(out) => {
            println!("{out}");
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
    let person = person()?;
    let current = find(&desired.name, &person)?;
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
                _ if desired.exist => choose(&desired, &person)?,
                _ => Place::System,
            };
            let mut s = state(&desired.name, desired.exist.then(|| (place, desired.entry(place))).as_ref());
            let what = place.what();
            let step = match (in_desired_state, desired.exist) {
                (true, true) => format!("{} is in {what}", desired.name),
                (true, false) => format!("{} is not in {what}", desired.name),
                (false, true) => format!("Add {} to {what} and rebuild it", desired.name),
                (false, false) => format!("Remove {} from {what} and rebuild it", desired.name),
            };
            let mut lines = vec![json!(step)];
            if desired.exist && !in_desired_state {
                let allowlist = match place {
                    Place::HomeStandalone => HomeTarget::find(&person)?.allowlist,
                    p => Target::here()?.allowlist(p, &person.name),
                };
                if let Err(e) = check_in(&desired.name, &desired.entry(place), &allowlist, place) {
                    lines.push(json!(format!("refused: {e:#}")));
                }
            }
            s["_metadata"] = json!({ "whatIf": lines });
            Ok(s)
        }
        "set" => {
            if !in_desired_state {
                // A changed app first leaves where it was, if it goes elsewhere now.
                let place = match &current {
                    Some((p, _)) if !desired.exist => *p,
                    _ => choose(&desired, &person)?,
                };
                if let Some((was, _)) = &current
                    && *was != place
                {
                    put(*was, &Desired { exist: false, ..desired.clone() }, &person)?;
                }
                put(place, &desired, &person)?;
            }
            Ok(state(&desired.name, find(&desired.name, &person)?.as_ref()))
        }
        _ => bail!("usage: get|test|set [--what-if]"),
    }
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
        let err = check("htop", &e, &allow).unwrap_err().to_string();
        assert!(err.contains("services.openssh.enable") && err.contains("allowedOptions"), "{err}");
        assert!(check("htop", &Entry { packages: vec!["a b".into()], ..Default::default() }, &allow).is_err());
        assert!(check("htop", &Entry { packages: vec!["${x}".into()], ..Default::default() }, &allow).is_err());
        assert!(check("a.b", &Entry::default(), &allow).is_err());
        let mut odd = Entry::default();
        odd.options.insert("programs.\"x\".enable".into(), json!(true));
        assert!(check("x", &odd, &["*.*.*".into()]).is_err());
    }

    #[test]
    fn a_configuration_is_rebuilt_only_for_its_owner_or_root() {
        let alice = |mode| -> Chain {
            vec![("/home/alice/cfg".into(), 1000, mode), ("/home/alice".into(), 1000, 0o700), ("/home".into(), 0, 0o755), ("/".into(), 0, 0o755)]
        };
        assert_eq!(may_rebuild(&alice(0o755), 1000).unwrap(), 1000);
        assert_eq!(may_rebuild(&alice(0o755), 0).unwrap(), 1000);
        let err = may_rebuild(&alice(0o755), 1001).unwrap_err().to_string();
        assert!(err.contains("another user"), "{err}");
        assert!(may_rebuild(&alice(0o775), 1000).is_err(), "the group can change it");
        let root: Chain = vec![("/etc/nixos".into(), 0, 0o755), ("/etc".into(), 0, 0o755), ("/".into(), 0, 0o755)];
        assert_eq!(may_rebuild(&root, 1001).unwrap(), 0);
        let under_bob: Chain = vec![("/home/bob/x/cfg".into(), 1000, 0o755), ("/home/bob/x".into(), 1001, 0o755), ("/".into(), 0, 0o755)];
        assert!(may_rebuild(&under_bob, 1000).is_err(), "bob could swap the folder");
        let tmp: Chain = vec![("/tmp/cfg".into(), 1000, 0o755), ("/tmp".into(), 0, 0o1777), ("/".into(), 0, 0o755)];
        assert!(may_rebuild(&tmp, 1000).is_err());
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
    fn the_home_setup_line_names_the_folder() {
        let line = home_setup_line(Path::new("/home/alice/.local/share/furnish/home-apps"));
        assert_eq!(line, "inputs.furnish.inputs.home-apps.url = \"path:/home/alice/.local/share/furnish/home-apps\";");
        assert_eq!(line_url(&line), "path:/home/alice/.local/share/furnish/home-apps");
        assert_eq!(home_apps_dir(Some(Path::new("/home/bob"))), Path::new("/home/bob/.local/share/furnish/home-apps"));
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
