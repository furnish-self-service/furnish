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
}

impl Default for Apps {
    fn default() -> Self {
        Apps { version: 1, apps: BTreeMap::new() }
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
            bail!("'{o}' isn't a NixOS option's name");
        }
        if !allowlist.iter().any(|a| allowed(a, o)) {
            bail!("the app sets the NixOS option {o}, which isn't in services.furnish.nix.allowedOptions on this computer");
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
    #[serde(rename = "_exist", default = "yes")]
    exist: bool,
}

fn yes() -> bool {
    true
}

impl Desired {
    fn entry(&self) -> Entry {
        Entry { packages: self.packages.clone(), options: self.options.clone() }
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
    if owner.uid != 0 {
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

/// Install (or remove, `desired.exist` false) an app in the configuration and rebuild.
fn apply(target: &Target, desired: &Desired) -> Result<()> {
    if !in_service() {
        bail!("only the furnish service changes the NixOS configuration: ask your administrator to turn it on (services.furnish.service.enable)");
    }
    let entry = desired.entry();
    if desired.exist {
        check(&desired.name, &entry, &target.facts.nix.allowed_options)?;
    }
    let requester: u32 = std::env::var("FURNISH_REQUESTER_UID").ok().and_then(|v| v.parse().ok()).unwrap_or(u32::MAX);
    let uid = may_rebuild(&chain(&target.dir)?, requester)?;
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
    let owner = account(&passwd, uid).with_context(|| format!("no account with the id {uid}"))?;

    let apps_dir = Path::new(APPS_DIR);
    let before = read_apps(apps_dir)?;
    let mut after = before.clone();
    if desired.exist {
        after.apps.insert(desired.name.clone(), entry);
    } else {
        after.apps.remove(&desired.name);
    }
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

    if clean {
        let verb = if desired.exist { "install" } else { "uninstall" };
        let mut commit = as_owner(&owner, "git");
        commit.arg("-C").arg(dir).args(["commit", "-q", "-m", &format!("furnish: {verb} {}", desired.name), "--", "flake.lock"]);
        match run(commit, "git commit") {
            Ok(_) => info(&format!("Committed flake.lock in {}", dir.display())),
            Err(e) => eprintln!("{}", json!({ "warn": format!("flake.lock changed, commit it yourself: {e:#}") })),
        }
    } else {
        info(&format!("{} has other changes: furnish leaves committing flake.lock to you", dir.display()));
    }
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

/// The state DSC sees: the app's entry in the local flake.
fn state(name: &str, entry: Option<&Entry>) -> Value {
    match entry {
        Some(e) => json!({ "name": name, "packages": e.packages, "options": e.options, "_exist": true }),
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
    let target = Target::here()?;
    let apps = read_apps(Path::new(APPS_DIR))?;
    let current = apps.apps.get(&desired.name);
    let in_desired_state = if desired.exist { current == Some(&desired.entry()) } else { current.is_none() };
    match action {
        "get" => Ok(state(&desired.name, current)),
        "test" => {
            let mut s = state(&desired.name, current);
            s["_inDesiredState"] = in_desired_state.into();
            Ok(s)
        }
        "set" if what_if => {
            let mut s = state(&desired.name, desired.exist.then(|| desired.entry()).as_ref());
            let step = match (in_desired_state, desired.exist) {
                (true, true) => format!("{} is in the NixOS configuration", desired.name),
                (true, false) => format!("{} is not in the NixOS configuration", desired.name),
                (false, true) => format!("Add {} to the NixOS configuration in {} and rebuild it", desired.name, target.dir.display()),
                (false, false) => format!("Remove {} from the NixOS configuration in {} and rebuild it", desired.name, target.dir.display()),
            };
            let mut lines = vec![json!(step)];
            if desired.exist
                && !in_desired_state
                && let Err(e) = check(&desired.name, &desired.entry(), &target.facts.nix.allowed_options)
            {
                lines.push(json!(format!("refused: {e:#}")));
            }
            s["_metadata"] = json!({ "whatIf": lines });
            Ok(s)
        }
        "set" => {
            if !in_desired_state {
                apply(&target, &desired)?;
            }
            let apps = read_apps(Path::new(APPS_DIR))?;
            Ok(state(&desired.name, apps.apps.get(&desired.name)))
        }
        _ => bail!("usage: get|test|set [--what-if]"),
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
