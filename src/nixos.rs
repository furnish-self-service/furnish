//! NixOS: where this computer's configuration lives, for the Settings page, `furnish settings`
//! and the resources (see `write_env`). Which packages the configuration installs is the
//! resource's business (resources/unix/furnish-nix.sh).
//!
//! furnish looks in this order and takes the first configuration that passes `check`:
//!
//! 1. the setting `osSpecific.nixos.configuration` (the user's, or the admin policy's);
//! 2. `/etc/nixos/flake.nix`, following a symlink, as nixos-rebuild does;
//! 3. nh's variables NH_OS_FLAKE, NH_FLAKE and FLAKE, in the environment or `/etc/set-environment`;
//! 4. `nixos-config=` in NIX_PATH, else `/etc/nixos/configuration.nix` (channels).
//!
//! Without one, folders where people often keep a flake (`~/nixos-config`, `~/dotfiles`, …) are
//! offered as suggestions; furnish never uses them on its own.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::Serialize;
use serde_json::{Map, Value};

use crate::settings::{Leaf, SetBy};

/// How furnish found a configuration or the flake attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    /// The user set it.
    Setting,
    /// The admin policy sets it.
    Policy,
    /// `/etc/nixos`.
    EtcNixos,
    /// nh's variables.
    Nh,
    /// `nixos-config=` in NIX_PATH.
    NixPath,
    /// The flake attribute: the computer's name.
    Hostname,
    /// Home Manager: HOME_MANAGER_CONFIG.
    Env,
    /// Home Manager: `~/.config/home-manager`.
    UsualPlace,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Source::Setting => "setting",
            Source::Policy => "policy",
            Source::EtcNixos => "etc-nixos",
            Source::Nh => "nh",
            Source::NixPath => "nix-path",
            Source::Hostname => "hostname",
            Source::Env => "env",
            Source::UsualPlace => "usual-place",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Flake,
    Channels,
}

/// Why a candidate doesn't pass `check`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Problem {
    /// No `flake.nix` or `configuration.nix` (Home Manager: `home.nix`) there.
    NotFound,
    /// Its `flake.nix` doesn't mention `nixosConfigurations`.
    NoNixosConfigurations,
    /// Its `flake.nix` doesn't mention the flake attribute (the computer's name by default).
    NoAttribute,
}

impl Problem {
    pub fn text(self, attribute: &str) -> String {
        match self {
            Problem::NotFound => "no flake.nix or configuration.nix there".into(),
            Problem::NoNixosConfigurations => "its flake.nix doesn't mention nixosConfigurations".into(),
            Problem::NoAttribute => format!("its flake.nix doesn't mention {attribute}"),
        }
    }
}

/// Whether the configuration's git commit is the one the running system was built from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Revision {
    /// The system doesn't say (no `system.configurationRevision`), or the folder isn't in git.
    Unknown,
    Matches,
    /// Shown as "doesn't match the running system"; still used.
    Differs,
}

/// Who owns a configuration folder. For later: a rebuild as root must not use a folder another
/// user can change on behalf of someone else (see `write_env`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Owner {
    pub uid: u32,
    pub user: Option<String>,
    /// Its group or everyone may write to it.
    pub others_can_write: bool,
}

/// A configuration furnish found.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    /// The folder (links resolved).
    pub path: PathBuf,
    /// The file in it: `flake.nix`, `configuration.nix`, `home.nix`, or what NIX_PATH names.
    pub file: PathBuf,
    pub kind: Kind,
    pub source: Source,
    /// The variable or path that led there, e.g. `NH_FLAKE` or `/etc/nixos/flake.nix`.
    pub via: Option<String>,
    pub owner: Option<Owner>,
    pub revision: Revision,
    /// For a configuration the user or the policy sets: what `check` would object to. furnish
    /// uses it anyway, because someone chose it.
    pub problem: Option<Problem>,
}

/// A folder with a `flake.nix` where people often keep their configuration.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub path: PathBuf,
    pub problem: Option<Problem>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attribute {
    pub value: String,
    pub source: Source,
}

/// The values the user or the policy set, as `settings::os_specific_leaf` gives them.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Chosen {
    pub configuration: Option<Leaf>,
    pub flake_attribute: Option<Leaf>,
    pub home_manager_configuration: Option<Leaf>,
}

/// How this NixOS computer is set up.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Setup {
    pub configuration: Option<Found>,
    pub flake_attribute: Attribute,
    pub home_manager_configuration: Option<Found>,
    /// This user has Home Manager (standalone or as a NixOS module).
    pub home_manager: bool,
    /// Only without a configuration.
    pub suggestions: Vec<Suggestion>,
    /// A configuration set that doesn't exist: the user's or the policy's path, and why.
    pub rejected: Option<(PathBuf, Problem)>,
    pub chosen: Chosen,
}

/// What `detect` looks at; the real computer, or fakes in tests.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// `/`, or a folder standing in for it.
    pub root: PathBuf,
    /// The user's home folder (without the root).
    pub home: Option<PathBuf>,
    pub user: String,
    /// The process environment.
    pub vars: BTreeMap<String, String>,
    pub chosen: Chosen,
    /// The furnish service: only the policy and what root owns count, never a user's settings.
    pub in_service: bool,
}

impl Context {
    /// This computer, as this process sees it. In the service, only the admin policy's values
    /// count: a root rebuild must never follow a path an ordinary user put in their settings.
    pub fn here() -> Context {
        let in_service = crate::service::in_service();
        let policy = crate::policy::load();
        let choices = if in_service { Map::new() } else { crate::settings::user_choices() };
        Context {
            root: PathBuf::from("/"),
            home: if in_service { None } else { dirs::home_dir() },
            user: std::env::var("USER").unwrap_or_default(),
            vars: std::env::vars().collect(),
            chosen: chosen(&policy, &choices),
            in_service,
        }
    }

    /// The real path of `p` (a path on the computer) under the root.
    fn fs(&self, p: &Path) -> PathBuf {
        self.root.join(p.strip_prefix("/").unwrap_or(p))
    }

    /// `p` (under the root) as a path on the computer.
    fn shown(&self, p: &Path) -> PathBuf {
        match p.strip_prefix(&self.root) {
            Ok(rest) => Path::new("/").join(rest),
            Err(_) => p.to_path_buf(),
        }
    }

    /// A variable from the environment, else as NixOS sets it for sessions (`/etc/set-environment`).
    fn var(&self, name: &str) -> Option<String> {
        if let Some(v) = self.vars.get(name).filter(|v| !v.is_empty()) {
            return Some(v.clone());
        }
        let text = std::fs::read_to_string(self.fs(Path::new("/etc/set-environment"))).ok()?;
        text.lines().find_map(|l| {
            let (k, v) = l.trim().strip_prefix("export ")?.split_once('=')?;
            let v = v.trim().trim_matches('"').trim_matches('\'');
            (k.trim() == name && !v.is_empty()).then(|| v.to_string())
        })
    }

    fn hostname(&self) -> String {
        ["/proc/sys/kernel/hostname", "/etc/hostname"]
            .iter()
            .find_map(|f| std::fs::read_to_string(self.fs(Path::new(f))).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()))
            .unwrap_or_else(|| "nixos".into())
    }

    /// `~/x` or `$HOME/x` → the home folder.
    fn expand(&self, p: &str) -> Option<PathBuf> {
        let rest = p.strip_prefix("~/").or_else(|| p.strip_prefix("$HOME/"));
        match rest {
            Some(rest) => self.home.as_ref().map(|h| h.join(rest)),
            None => Some(PathBuf::from(p)),
        }
    }
}

/// The values set for `osSpecific.nixos`, from the policy and the user's choices.
pub fn chosen(policy: &crate::policy::Policy, choices: &Map<String, Value>) -> Chosen {
    let mut ignore = Vec::new();
    let mut leaf = |name| crate::settings::os_specific_leaf(policy, choices, "nixos", name, &mut ignore);
    Chosen { configuration: leaf("configuration"), flake_attribute: leaf("flakeAttribute"), home_manager_configuration: leaf("homeManagerConfiguration") }
}

/// `None` when this isn't NixOS. Compares the configuration's git commit with the running system.
pub fn setup() -> Option<Setup> {
    let ctx = Context::here();
    let mut setup = detect(&ctx)?;
    if let Some(found) = &mut setup.configuration {
        found.revision = revision(&ctx, &found.path, running_revision().as_deref());
    }
    Some(setup)
}

/// Where the configuration is, without comparing commits (no process runs). `None`: not NixOS.
pub fn detect(ctx: &Context) -> Option<Setup> {
    if !ctx.fs(Path::new("/etc/NIXOS")).exists() {
        return None;
    }
    let flake_attribute = match &ctx.chosen.flake_attribute {
        Some(l) => Attribute { value: l.value.as_str().unwrap_or_default().to_string(), source: source_of(l) },
        None => Attribute { value: ctx.hostname(), source: Source::Hostname },
    };
    let attr = flake_attribute.value.as_str();
    let mut rejected = None;
    let mut configuration = None;
    // 1. Set by the user or the policy: used if the file is there, even if the text check fails.
    if let Some(l) = &ctx.chosen.configuration {
        let path = PathBuf::from(l.value.as_str().unwrap_or_default());
        match check(ctx, &path, attr, false) {
            Ok(mut f) => {
                f.problem = check(ctx, &path, attr, true).err();
                configuration = Some(Found { source: source_of(l), ..f });
            }
            Err(p) => rejected = Some((path, p)),
        }
    }
    // 2. /etc/nixos/flake.nix, through a symlink.
    if configuration.is_none() {
        configuration = found(ctx, Path::new("/etc/nixos/flake.nix"), attr, Source::EtcNixos, Some("/etc/nixos/flake.nix".into()));
    }
    // 3. nh.
    if configuration.is_none() {
        configuration = ["NH_OS_FLAKE", "NH_FLAKE", "FLAKE"]
            .into_iter()
            .find_map(|var| found(ctx, &flake_path(ctx, &ctx.var(var)?)?, attr, Source::Nh, Some(var.into())));
    }
    // 4. Channels: NIX_PATH's nixos-config, else nixos-rebuild's default.
    if configuration.is_none() {
        let from_path = ctx.var("NIX_PATH").and_then(|p| {
            p.split(':').find_map(|e| e.strip_prefix("nixos-config=").and_then(|v| ctx.expand(v)))
        });
        configuration = match from_path {
            Some(p) => found(ctx, &p, attr, Source::NixPath, Some("NIX_PATH".into())),
            None => found(ctx, Path::new("/etc/nixos/configuration.nix"), attr, Source::EtcNixos, None),
        };
        if let Some(f) = &mut configuration
            && f.path.starts_with("/etc/nixos")
        {
            f.source = Source::EtcNixos;
        }
    }
    let suggestions = match (&configuration, &ctx.home) {
        (None, Some(home)) if !ctx.in_service => ["nixos-config", ".config/nixos", "dotfiles", ".dotfiles", "nixos", "cfg"]
            .iter()
            .map(|d| home.join(d))
            .filter(|d| ctx.fs(&d.join("flake.nix")).is_file())
            .map(|d| Suggestion { problem: check(ctx, &d, attr, true).err(), path: d })
            .collect(),
        _ => Vec::new(),
    };
    Some(Setup {
        configuration,
        home_manager_configuration: home_manager(ctx),
        home_manager: has_home_manager(ctx),
        flake_attribute,
        suggestions,
        rejected,
        chosen: ctx.chosen.clone(),
    })
}

fn source_of(l: &Leaf) -> Source {
    match l.set_by {
        SetBy::User => Source::Setting,
        SetBy::Organization => Source::Policy,
    }
}

fn found(ctx: &Context, path: &Path, attr: &str, source: Source, via: Option<String>) -> Option<Found> {
    check(ctx, path, attr, true).ok().map(|f| Found { source, via, ..f })
}

/// A local path from a flake reference such as `/home/alex/cfg`, `path:/x`, `git+file:///x?ref=main`
/// or `~/cfg#laptop`; `None` for remote ones (`github:…`).
fn flake_path(ctx: &Context, reference: &str) -> Option<PathBuf> {
    let r = reference.split(['#', '?']).next()?;
    let r = r.strip_prefix("git+file://").or_else(|| r.strip_prefix("path:")).unwrap_or(r);
    (r.starts_with('/') || r.starts_with('~') || r.starts_with("$HOME")).then(|| ctx.expand(r)).flatten()
}

/// Whether there is a NixOS configuration at `path` (a folder, or the file itself): a `flake.nix`
/// that mentions `nixosConfigurations` and the attribute (with `text`), or a `configuration.nix`
/// (or the `.nix` file NIX_PATH names). Cheap: reads one file, evaluates nothing.
pub fn check(ctx: &Context, path: &Path, attr: &str, text: bool) -> Result<Found, Problem> {
    let real = ctx.fs(path);
    let real = real.canonicalize().map_err(|_| Problem::NotFound)?;
    let file = if real.is_dir() {
        ["flake.nix", "configuration.nix", "default.nix"].iter().map(|f| real.join(f)).find(|f| f.is_file()).ok_or(Problem::NotFound)?
    } else if real.extension().is_some_and(|e| e == "nix") {
        real
    } else {
        return Err(Problem::NotFound);
    };
    let kind = if file.ends_with("flake.nix") { Kind::Flake } else { Kind::Channels };
    if kind == Kind::Flake && text {
        let content = std::fs::read_to_string(&file).map_err(|_| Problem::NotFound)?;
        if !content.contains("nixosConfigurations") {
            return Err(Problem::NoNixosConfigurations);
        }
        if !mentions(&content, attr) {
            return Err(Problem::NoAttribute);
        }
    }
    let folder = file.parent().unwrap_or(&file).to_path_buf();
    Ok(Found {
        owner: owner(ctx, &folder),
        path: ctx.shown(&folder),
        file: ctx.shown(&file),
        kind,
        source: Source::Setting,
        via: None,
        revision: Revision::Unknown,
        problem: None,
    })
}

/// `name` appears in `text` as a whole name (not inside a longer one).
fn mentions(text: &str, name: &str) -> bool {
    let part = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '\'';
    !name.is_empty()
        && text.match_indices(name).any(|(i, _)| {
            !text[..i].chars().next_back().is_some_and(part) && !text[i + name.len()..].chars().next().is_some_and(part)
        })
}

#[cfg(unix)]
fn owner(ctx: &Context, folder: &Path) -> Option<Owner> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(folder).ok()?;
    let uid = meta.uid();
    let passwd = std::fs::read_to_string(ctx.fs(Path::new("/etc/passwd"))).unwrap_or_default();
    let user = passwd.lines().find_map(|l| {
        let mut f = l.split(':');
        let name = f.next()?;
        (f.nth(1)?.parse::<u32>().ok()? == uid).then(|| name.to_string())
    });
    Some(Owner { uid, user, others_can_write: meta.mode() & 0o022 != 0 })
}

#[cfg(not(unix))]
fn owner(_: &Context, _: &Path) -> Option<Owner> {
    None
}

/// Home Manager's configuration: the setting, HOME_MANAGER_CONFIG, `~/.config/home-manager`.
fn home_manager(ctx: &Context) -> Option<Found> {
    let at = |path: &Path, source, via: Option<String>| -> Option<Found> {
        let real = ctx.fs(path).canonicalize().ok()?;
        let file = if real.is_dir() { ["flake.nix", "home.nix"].iter().map(|f| real.join(f)).find(|f| f.is_file())? } else { real };
        let folder = file.parent()?.to_path_buf();
        Some(Found {
            kind: if file.ends_with("flake.nix") { Kind::Flake } else { Kind::Channels },
            owner: owner(ctx, &folder),
            path: ctx.shown(&folder),
            file: ctx.shown(&file),
            source,
            via,
            revision: Revision::Unknown,
            problem: None,
        })
    };
    if let Some(l) = &ctx.chosen.home_manager_configuration
        && let Some(f) = at(Path::new(l.value.as_str().unwrap_or_default()), source_of(l), None)
    {
        return Some(f);
    }
    if ctx.in_service {
        return None;
    }
    if let Some(f) = ctx.vars.get("HOME_MANAGER_CONFIG").and_then(|v| ctx.expand(v)).and_then(|p| at(&p, Source::Env, Some("HOME_MANAGER_CONFIG".into()))) {
        return Some(f);
    }
    at(&ctx.home.as_ref()?.join(".config/home-manager"), Source::UsualPlace, None)
}

fn has_home_manager(ctx: &Context) -> bool {
    if ctx.user.is_empty() {
        return false;
    }
    // Its generations: in the user's state folder, or (older Home Manager) in Nix's per-user profiles.
    let mut generations = vec![ctx.fs(Path::new("/nix/var/nix/profiles/per-user")).join(&ctx.user).join("home-manager")];
    if let Some(home) = &ctx.home {
        generations.push(ctx.fs(&home.join(".local/state/nix/profiles/home-manager")));
        generations.push(ctx.fs(&home.join(".config/home-manager")));
    }
    generations.iter().any(|p| p.exists())
}

/// The commit the running system was built from (`system.configurationRevision`), if it says.
fn running_revision() -> Option<String> {
    let out = std::process::Command::new("nixos-version").arg("--json").output().ok()?;
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    v["configurationRevision"].as_str().map(String::from)
}

/// Compare the folder's git HEAD with the system's revision (a `-dirty` suffix doesn't count).
pub fn revision(ctx: &Context, folder: &Path, running: Option<&str>) -> Revision {
    let Some(running) = running.map(|r| r.trim_end_matches("-dirty").trim_start_matches("dirty-")).filter(|r| r.len() >= 7) else {
        return Revision::Unknown;
    };
    let Some(head) = git_head(&ctx.fs(folder)) else { return Revision::Unknown };
    if head.starts_with(running) || running.starts_with(&head) { Revision::Matches } else { Revision::Differs }
}

/// The commit checked out in the git repository containing `folder`, read from its files.
fn git_head(folder: &Path) -> Option<String> {
    let git = folder.ancestors().map(|d| d.join(".git")).find(|g| g.is_dir())?;
    let head = std::fs::read_to_string(git.join("HEAD")).ok()?;
    let head = head.trim();
    let Some(r) = head.strip_prefix("ref: ") else { return Some(head.to_string()) };
    if let Ok(id) = std::fs::read_to_string(git.join(r)) {
        return Some(id.trim().to_string());
    }
    let packed = std::fs::read_to_string(git.join("packed-refs")).ok()?;
    packed.lines().find_map(|l| l.strip_suffix(r).map(|id| id.trim().to_string()))
}

/// What furnish found, for the resources (see `resource_env`): `osSpecific.nixos.*` as
/// FURNISH_DETECTED__OS_SPECIFIC__NIXOS__*; `null` (an empty variable) when unknown.
///
/// Security: in the furnish service, `Context::here` takes the configuration only from the
/// admin policy or root's own places (/etc/nixos, /etc/set-environment, the service's
/// environment), never from a user's settings. The folder can still belong to a user (e.g.
/// /etc/nixos/flake.nix linked to /home/alice/cfg): `configurationOwner` (`<uid>:<user>`) and
/// `configurationOthersCanWrite` say so, so a later rebuild as root can refuse to build a
/// configuration that isn't root's on behalf of another user.
pub fn detected(setup: Option<&Setup>) -> Value {
    let c = setup.and_then(|s| s.configuration.as_ref());
    let owner = c.and_then(|f| f.owner.as_ref());
    serde_json::json!({ "osSpecific": { "nixos": {
        "configuration": c.map(|f| f.path.display().to_string()),
        "configurationSource": c.map(|f| f.source.name()),
        "configurationKind": c.map(|f| if f.kind == Kind::Flake { "flake" } else { "channels" }),
        "configurationOwner": owner.map(|o| format!("{}:{}", o.uid, o.user.as_deref().unwrap_or_default())),
        "configurationOthersCanWrite": owner.map(|o| o.others_can_write),
        "flakeAttribute": setup.map(|s| s.flake_attribute.value.clone()),
        "homeManagerConfiguration": setup.and_then(|s| s.home_manager_configuration.as_ref()).map(|f| f.path.display().to_string()),
    }}})
}

/// `detected` for this computer (nothing found off NixOS); runs no process.
pub fn detected_here() -> Value {
    let setup = if Path::new("/etc/NIXOS").exists() { detect(&Context::here()) } else { None };
    detected(setup.as_ref())
}

// The fakes use symlinks and uids.
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;

    struct Fake {
        root: PathBuf,
    }

    impl Fake {
        fn new(name: &str) -> Fake {
            let root = std::env::temp_dir().join(format!("furnish-nixos-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("etc/nixos")).unwrap();
            fs::create_dir_all(root.join("home/alice")).unwrap();
            fs::write(root.join("etc/NIXOS"), "").unwrap();
            fs::create_dir_all(root.join("proc/sys/kernel")).unwrap();
            fs::write(root.join("proc/sys/kernel/hostname"), "laptop\n").unwrap();
            Fake { root }
        }
        fn ctx(&self) -> Context {
            Context { root: self.root.clone(), home: Some("/home/alice".into()), user: "alice".into(), ..Default::default() }
        }
        fn flake(&self, dir: &str, text: &str) {
            let d = self.root.join(dir.trim_start_matches('/'));
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("flake.nix"), text).unwrap();
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    const LAPTOP: &str = "{ outputs = { nixpkgs, ... }: { nixosConfigurations.laptop = nixpkgs.lib.nixosSystem { }; }; }";

    fn leaf(v: &str, set_by: SetBy, locked: bool) -> Option<Leaf> {
        Some(Leaf { value: v.into(), set_by, locked })
    }

    #[test]
    fn not_nixos_without_etc_nixos_marker() {
        let f = Fake::new("marker");
        fs::remove_file(f.root.join("etc/NIXOS")).unwrap();
        assert_eq!(detect(&f.ctx()), None);
    }

    #[test]
    fn etc_nixos_flake_through_a_symlink() {
        let f = Fake::new("symlink");
        f.flake("/home/alice/cfg", LAPTOP);
        std::os::unix::fs::symlink(f.root.join("home/alice/cfg/flake.nix"), f.root.join("etc/nixos/flake.nix")).unwrap();
        let s = detect(&f.ctx()).unwrap();
        let c = s.configuration.unwrap();
        assert_eq!((c.path.as_path(), c.source, c.kind), (Path::new("/home/alice/cfg"), Source::EtcNixos, Kind::Flake));
        assert_eq!(s.flake_attribute, Attribute { value: "laptop".into(), source: Source::Hostname });
        assert!(s.suggestions.is_empty());
    }

    #[test]
    fn the_setting_comes_first_and_the_attribute_must_be_mentioned_elsewhere() {
        let f = Fake::new("setting");
        f.flake("/etc/nixos", LAPTOP);
        f.flake("/home/alice/mine", "{ nixosConfigurations.desk = 1; }");
        let mut ctx = f.ctx();
        // Mentions another host only: /etc/nixos (which mentions laptop) wins over nh.
        ctx.vars.insert("NH_FLAKE".into(), "/home/alice/mine".into());
        assert_eq!(detect(&ctx).unwrap().configuration.unwrap().source, Source::EtcNixos);
        // The user's choice is used even though the text check fails; the problem is shown.
        ctx.chosen.configuration = leaf("/home/alice/mine", SetBy::User, false);
        let c = detect(&ctx).unwrap().configuration.unwrap();
        assert_eq!((c.path.as_path(), c.source, c.problem), (Path::new("/home/alice/mine"), Source::Setting, Some(Problem::NoAttribute)));
        // With the attribute set, it passes; the policy's choice says so.
        ctx.chosen.flake_attribute = leaf("desk", SetBy::Organization, true);
        ctx.chosen.configuration = leaf("/home/alice/mine", SetBy::Organization, true);
        let s = detect(&ctx).unwrap();
        assert_eq!((s.configuration.as_ref().unwrap().source, s.configuration.unwrap().problem), (Source::Policy, None));
        assert_eq!(s.flake_attribute, Attribute { value: "desk".into(), source: Source::Policy });
        // A path that doesn't exist is rejected and the search goes on.
        ctx.chosen.configuration = leaf("/home/alice/gone", SetBy::User, false);
        ctx.chosen.flake_attribute = None;
        let s = detect(&ctx).unwrap();
        assert_eq!(s.rejected, Some(("/home/alice/gone".into(), Problem::NotFound)));
        assert_eq!(s.configuration.unwrap().source, Source::EtcNixos);
    }

    #[test]
    fn nh_variables_in_order_and_from_set_environment() {
        let f = Fake::new("nh");
        f.flake("/home/alice/a", LAPTOP);
        f.flake("/home/alice/b", LAPTOP);
        let mut ctx = f.ctx();
        ctx.vars.insert("FLAKE".into(), "/home/alice/b".into());
        ctx.vars.insert("NH_FLAKE".into(), "git+file:///home/alice/a?ref=main".into());
        let c = detect(&ctx).unwrap().configuration.unwrap();
        assert_eq!((c.path.as_path(), c.source, c.via.as_deref()), (Path::new("/home/alice/a"), Source::Nh, Some("NH_FLAKE")));
        // Remote references don't count; NixOS's session variables do.
        ctx.vars.clear();
        ctx.vars.insert("NH_OS_FLAKE".into(), "github:alice/cfg".into());
        fs::write(f.root.join("etc/set-environment"), "export PATH=\"/x\"\nexport NH_FLAKE=\"~/b#laptop\"\n").unwrap();
        let c = detect(&ctx).unwrap().configuration.unwrap();
        assert_eq!((c.path.as_path(), c.via.as_deref()), (Path::new("/home/alice/b"), Some("NH_FLAKE")));
    }

    #[test]
    fn channels_from_nix_path_or_etc_nixos() {
        let f = Fake::new("channels");
        fs::write(f.root.join("etc/nixos/configuration.nix"), "{ }").unwrap();
        let mut ctx = f.ctx();
        let c = detect(&ctx).unwrap().configuration.unwrap();
        assert_eq!((c.file.as_path(), c.source, c.kind), (Path::new("/etc/nixos/configuration.nix"), Source::EtcNixos, Kind::Channels));
        fs::create_dir_all(f.root.join("home/alice/sys")).unwrap();
        fs::write(f.root.join("home/alice/sys/machine.nix"), "{ }").unwrap();
        ctx.vars.insert("NIX_PATH".into(), "nixpkgs=/x:nixos-config=/home/alice/sys/machine.nix".into());
        let c = detect(&ctx).unwrap().configuration.unwrap();
        assert_eq!((c.file.as_path(), c.source), (Path::new("/home/alice/sys/machine.nix"), Source::NixPath));
    }

    #[test]
    fn suggestions_only_without_a_configuration_and_never_used() {
        let f = Fake::new("suggest");
        f.flake("/home/alice/dotfiles", LAPTOP);
        f.flake("/home/alice/nixos", "{ }");
        let mut ctx = f.ctx();
        let s = detect(&ctx).unwrap();
        assert_eq!(s.configuration, None);
        assert_eq!(
            s.suggestions,
            [
                Suggestion { path: "/home/alice/dotfiles".into(), problem: None },
                Suggestion { path: "/home/alice/nixos".into(), problem: Some(Problem::NoNixosConfigurations) }
            ]
        );
        ctx.in_service = true;
        assert!(detect(&ctx).unwrap().suggestions.is_empty());
    }

    #[test]
    fn check_matches_whole_names_only() {
        assert!(mentions("nixosConfigurations.laptop = x", "laptop"));
        assert!(mentions("nixosConfigurations = { \"laptop\" = x; }", "laptop"));
        assert!(!mentions("nixosConfigurations.laptop2 = x", "laptop"));
        assert!(!mentions("my-laptop", "laptop"));
    }

    #[test]
    fn revision_matches_or_differs() {
        let f = Fake::new("revision");
        f.flake("/home/alice/cfg/hosts", LAPTOP);
        let git = f.root.join("home/alice/cfg/.git");
        fs::create_dir_all(git.join("refs/heads")).unwrap();
        fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        let id = "0123456789abcdef0123456789abcdef01234567";
        fs::write(git.join("refs/heads/main"), format!("{id}\n")).unwrap();
        let ctx = f.ctx();
        let folder = Path::new("/home/alice/cfg/hosts");
        assert_eq!(revision(&ctx, folder, Some(id)), Revision::Matches);
        assert_eq!(revision(&ctx, folder, Some(&format!("{id}-dirty"))), Revision::Matches);
        assert_eq!(revision(&ctx, folder, Some("fedcba9876543210")), Revision::Differs);
        assert_eq!(revision(&ctx, folder, None), Revision::Unknown);
        // Packed refs.
        fs::remove_file(git.join("refs/heads/main")).unwrap();
        fs::write(git.join("packed-refs"), format!("# pack-refs\n{id} refs/heads/main\n")).unwrap();
        assert_eq!(revision(&ctx, folder, Some(&id[..12])), Revision::Matches);
    }

    #[test]
    fn home_manager_setting_env_or_usual_place() {
        let f = Fake::new("hm");
        let mut ctx = f.ctx();
        assert_eq!(detect(&ctx).unwrap().home_manager_configuration, None);
        fs::create_dir_all(f.root.join("home/alice/.config/home-manager")).unwrap();
        fs::write(f.root.join("home/alice/.config/home-manager/home.nix"), "{ }").unwrap();
        let hm = detect(&ctx).unwrap().home_manager_configuration.unwrap();
        assert_eq!((hm.path.as_path(), hm.source), (Path::new("/home/alice/.config/home-manager"), Source::UsualPlace));
        f.flake("/home/alice/hm", "{ }");
        ctx.vars.insert("HOME_MANAGER_CONFIG".into(), "/home/alice/hm/flake.nix".into());
        assert_eq!(detect(&ctx).unwrap().home_manager_configuration.unwrap().source, Source::Env);
        ctx.chosen.home_manager_configuration = leaf("/home/alice/.config/home-manager", SetBy::User, false);
        assert_eq!(detect(&ctx).unwrap().home_manager_configuration.unwrap().source, Source::Setting);
    }

    #[test]
    fn detected_values_for_the_resources() {
        let f = Fake::new("env");
        f.flake("/home/alice/cfg", LAPTOP);
        std::os::unix::fs::symlink(f.root.join("home/alice/cfg/flake.nix"), f.root.join("etc/nixos/flake.nix")).unwrap();
        fs::write(f.root.join("etc/passwd"), format!("root:x:0:0::/root:/bin/sh\nalice:x:{}:100::/home/alice:/bin/sh\n", unsafe { libc::getuid() })).unwrap();
        let s = detect(&f.ctx()).unwrap();
        let d = &detected(Some(&s))["osSpecific"]["nixos"];
        assert_eq!(d["configuration"], "/home/alice/cfg");
        assert_eq!(d["configurationKind"], "flake");
        assert_eq!(d["configurationSource"], "etc-nixos");
        assert_eq!(d["flakeAttribute"], "laptop");
        assert!(d["configurationOwner"].as_str().unwrap().ends_with(":alice"));
        assert_eq!(d["homeManagerConfiguration"], Value::Null);
        // Not NixOS: nothing known.
        assert!(detected(None)["osSpecific"]["nixos"].as_object().unwrap().values().all(Value::is_null));
    }

    #[test]
    fn the_service_ignores_the_users_settings() {
        let mut policy = crate::policy::Policy::default();
        policy.os_specific.insert("nixos".into(), [("flakeAttribute".to_string(), serde_json::json!("desk"))].into());
        let choices = serde_json::json!({"osSpecific": {"nixos": {"configuration": "/home/alice/cfg"}}});
        let c = chosen(&policy, choices.as_object().unwrap());
        assert_eq!(c.configuration.unwrap().set_by, SetBy::User);
        // What Context::here passes in the service: the policy only.
        let c = chosen(&policy, &Map::new());
        assert_eq!((c.configuration, c.flake_attribute.unwrap().set_by), (None, SetBy::Organization));
    }
}
