//! App collections: git repositories with one DSC configuration document per app in `apps/`.

use std::{
    borrow::Cow,
    collections::BTreeMap,
    env, fs,
    io::{Seek, Write},
    path::{Component, Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use serde_json::Value;

mod release;
pub use release::{AppChange, Change, CollectionPreview, ReviewedRevisions, StepChange};
mod tracking;
mod status_cache;
pub(crate) mod flake;
use crate::settings::Tracking;
use tracking::Target;

pub(crate) const DSC_SCHEMA: &str = "https://aka.ms/dsc/schemas/v3/bundled/config/document.json";

pub struct Collection {
    pub name: String,
    pub dir: PathBuf,
    /// Preset by the admin policy: users can't remove it.
    pub managed: bool,
}

/// A curated set of apps from the same collection, installed together (`kits/<id>.yaml`).
pub struct Kit {
    pub id: String,
    pub collection: String,
    /// The name to show (`name`), if other than the id.
    pub name: Option<String>,
    pub description: String,
    pub categories: Vec<String>,
    pub icon: Option<PathBuf>,
    /// Shown instead of `icon` in dark mode (`iconDark`).
    pub icon_dark: Option<PathBuf>,
    pub items: Vec<Item>,
    pub(crate) recommendation: Value,
    collection_dir: PathBuf,
}

impl Kit {
    pub fn recommended(&self) -> bool {
        recommended(&self.recommendation, &self.collection, &self.collection_dir, None)
    }
}

fn recommended(rule: &Value, collection: &str, dir: &Path, app: Option<&App>) -> bool {
    match rule {
        Value::Bool(b) => *b,
        Value::String(expr) => {
            let shared = crate::options::collection_declarations(dir).unwrap_or_default();
            let mut values = crate::options::values(&crate::options::resolve_collection(collection, &shared));
            if let Some(app) = app {
                values.extend(app.current_values());
            }
            // An unanswered option makes the expression fail until an answer is available.
            crate::conditions::holds_for(app.map_or(&Value::Null, |a| &a.doc), expr, &values).unwrap_or(false)
        }
        _ => false,
    }
}

/// An app a kit contains, or an app depends on.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    /// App id in the same collection.
    pub app: String,
    /// Always installed; users can't deselect it.
    #[serde(default)]
    pub required: bool,
    /// For optional items: preselected when the user installs.
    #[serde(default = "yes")]
    pub selected: bool,
    /// A dependency applies when this DSC expression over the host and app parameters is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
}

/// What installing starts with: required items and optional items selected by default.
pub fn default_selection(items: &[Item]) -> Vec<&str> {
    items.iter().filter(|i| i.required || i.selected).map(|i| i.app.as_str()).collect()
}

/// `default_selection` plus `with`, minus `without`; fails for unknown items or a required one in `without`.
pub fn choose<'a>(items: &'a [Item], with: &'a [String], without: &[String], what: &str) -> Result<Vec<&'a str>> {
    for name in with.iter().chain(without) {
        if !items.iter().any(|i| &i.app == name) {
            bail!("'{name}' is not part of {what}");
        }
    }
    if let Some(required) = items.iter().find(|i| i.required && without.contains(&i.app)) {
        bail!("'{}' is required by {what}", required.app);
    }
    let mut chosen = default_selection(items);
    chosen.extend(with.iter().map(String::as_str));
    chosen.retain(|app| !without.iter().any(|w| w == app));
    chosen.dedup();
    Ok(chosen)
}

fn yes() -> bool {
    true
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct KitFile {
    name: Option<String>,
    #[serde(default)]
    description: String,
    #[serde(default)]
    categories: Vec<String>,
    icon: Option<String>,
    #[serde(rename = "iconDark")]
    icon_dark: Option<String>,
    items: Vec<Item>,
    #[serde(default)]
    recommended: Value,
}

#[derive(Clone)]
pub struct App {
    /// Namespace of the originating collection's package ownership records.
    pub(crate) package_scope: String,
    pub id: String,
    pub collection: String,
    /// The name to show (`metadata.furnish.name`, e.g. "Google Chrome"); the id stays the file
    /// name, which commands, policies and dependencies use.
    pub name: Option<String>,
    pub description: String,
    pub categories: Vec<String>,
    /// Optional icon image (`metadata.furnish.icon`, relative to the collection root).
    pub icon: Option<PathBuf>,
    /// Shown instead of `icon` in dark mode (`metadata.furnish.iconDark`).
    pub icon_dark: Option<PathBuf>,
    /// The app's DSC configuration file.
    pub path: PathBuf,
    /// Apps of the same collection installed first (`metadata.furnish.dependencies`), e.g. Homebrew.
    pub dependencies: Vec<Item>,
    /// The collection's directory (for `collection.yaml`).
    pub collection_dir: PathBuf,
    /// From a collection the admin policy presets.
    pub managed: bool,
    pub doc: Value,
    /// Held at this earlier version, because the collection's current one failed or the person
    /// chose it (`chosen`); `doc` is that version's (see `versions`).
    pub held: Option<crate::versions::Held>,
}

/// Which version of an app to install.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Which {
    /// The version furnish shows: the collection's, or the one the app is held at.
    #[default]
    Shown,
    /// The collection's version, also for a held app.
    Current,
    /// The earlier version to fall back to (see `App::previous_version`).
    Previous,
    /// A version the person chose (a commit, see `App::versions`); the app stays there until they
    /// choose another one.
    Version(String),
}

/// A version of an app: a commit of its collection that changed the app's file.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Version {
    pub commit: String,
    /// The day of the commit, e.g. 2026-11-02.
    pub date: String,
    /// The versions the app's file lists as acceptable at this commit (`versions` of its first
    /// resource that has them), if it lists any.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stated: Vec<crate::required::Entry>,
    /// The collection's current version of the app.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub current: bool,
}

/// Data directory; `FURNISH_DATA_DIR` overrides the OS default.
pub fn data_dir() -> PathBuf {
    env::var_os("FURNISH_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::data_dir().expect("no data directory on this OS").join("furnish"))
}

/// Data directory of the furnish service (see `service`); `FURNISH_SYSTEM_DIR` overrides it.
pub fn system_dir() -> PathBuf {
    if let Some(dir) = env::var_os("FURNISH_SYSTEM_DIR") {
        return dir.into();
    }
    if cfg!(windows) {
        PathBuf::from(env::var_os("ProgramData").unwrap_or_else(|| r"C:\ProgramData".into())).join("furnish")
    } else if cfg!(target_os = "macos") {
        PathBuf::from("/Library/Application Support/furnish")
    } else {
        PathBuf::from("/var/lib/furnish")
    }
}

fn collections_dir() -> PathBuf {
    data_dir().join("collections")
}

/// With the furnish service, the preset collections are its copies, which users can read but not change.
fn preset_dir() -> PathBuf {
    if crate::service::enabled() { system_dir().join("collections") } else { collections_dir() }
}

pub fn collections() -> Result<Vec<Collection>> {
    let policy = crate::policy::load().collections;
    let preset_name = |name: &str| policy.preset.iter().any(|p| p.name == name);
    let in_dir = |dir: PathBuf| -> Result<Vec<Collection>> {
        if !dir.exists() {
            return Ok(vec![]);
        }
        Ok(fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().join(".git").exists())
            .map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                Collection { managed: is_preset(&policy.preset, &name, &e.path()), name, dir: e.path() }
            })
            .collect())
    };
    let separate = preset_dir() != collections_dir();
    let mut collections: Vec<_> = in_dir(collections_dir())?
        .into_iter()
        // With the service, a preset's name belongs to the service's copy. Personal copies
        // excluded by current policy stay on disk and become available again if allowed.
        .filter(|c| !separate || !preset_name(&c.name))
        .filter(|c| c.managed || (policy.users_can_add && personal_origin_allowed(&policy, c)))
        .collect();
    if separate {
        collections.extend(in_dir(preset_dir())?.into_iter().filter(|c| c.managed));
    }
    collections.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(collections)
}

/// A preset collection: the policy presets its name, and the copy comes from the URL the policy
/// gives for it, compared exactly as written. A collection of the same name from elsewhere is not.
fn is_preset(presets: &[crate::policy::Preset], name: &str, dir: &Path) -> bool {
    presets.iter().any(|p| p.name == name && origin_urls_match(dir, |url| url == p.url))
}

fn origin_url(dir: &Path) -> Result<String> {
    git(Some(dir), &["config", "--get", "remote.origin.url"])
}

/// Git can have multiple URLs for a remote. Check them all, including the first one Git fetches.
fn origin_urls_match(dir: &Path, allowed: impl Fn(&str) -> bool) -> bool {
    let Ok(bytes) = git_bytes(Some(dir), &["config", "--local", "--null", "--get-all", "remote.origin.url"]) else {
        return false;
    };
    let Ok(urls) = std::str::from_utf8(&bytes) else { return false };
    urls.strip_suffix('\0').is_some_and(|list| list.split('\0').all(|url| !url.is_empty() && allowed(url)))
}

fn personal_origin_allowed(policy: &crate::policy::Collections, collection: &Collection) -> bool {
    !policy.has_user_url_restriction() || (
        origin_urls_match(&collection.dir, |url| policy.check_user_url(url).is_ok())
        && collection.url().is_ok_and(|url| policy.check_user_url(&url).is_ok())
    )
}

/// Also check Git's fetch destination when an insteadOf rule or remote name rewrites the URL.
/// --get-url resolves it locally, before any credentials are requested or connection is made.
pub(crate) fn check_git_url(policy: &crate::policy::Collections, url: &str) -> Result<()> {
    if policy.has_user_url_restriction() {
        policy.check_user_url(url)?;
        let destination = git(None, &["ls-remote", "--get-url", "--end-of-options", url])
            .map_err(|_| anyhow::anyhow!("could not resolve the collection URL for your organization's policy"))?;
        policy.check_user_url(&destination)?;
    }
    Ok(())
}

pub fn get(name: &str) -> Result<Collection> {
    collections()?
        .into_iter()
        .find(|c| c.name == name)
        .with_context(|| format!("no collection named '{name}'"))
}

/// The branding logo (`branding.logo` in the policy): an absolute path, or `<collection>/<path>`
/// inside a preset collection. `None` if missing or outside that collection.
pub fn branding_logo() -> Option<PathBuf> {
    branding_file(crate::policy::load().branding.logo?)
}

/// The policy's `branding.trayIcon`, found like `branding_logo`.
pub fn branding_tray_icon() -> Option<PathBuf> {
    branding_file(crate::policy::load().branding.tray_icon?)
}

fn branding_file(spec: String) -> Option<PathBuf> {
    if Path::new(&spec).is_absolute() {
        return Path::new(&spec).is_file().then(|| PathBuf::from(spec));
    }
    let (name, path) = spec.split_once('/')?;
    // Only from the organization's own collections: preset ones, or those of its endpoints.
    let theirs = |c: &Collection| c.managed || c.url().is_ok_and(|u| crate::endpoint::for_url(&u).is_some());
    get(name).ok().filter(theirs)?.file_in_repo(path)
}

/// Default collection name: last URL path segment without `.git`.
pub fn name_from_url(url: &str) -> String {
    let last = url.trim_end_matches('/').rsplit(['/', ':']).next().unwrap_or(url);
    last.trim_end_matches(".git").to_string()
}

/// Add a collection the user chose; the admin policy may forbid that. `credentials`: a username
/// and password (or access token) for an HTTPS URL, kept in the OS's credential store.
pub fn add(url: &str, name: &str, credentials: Option<(&str, &str)>) -> Result<Collection> {
    add_with_tracking(url, name, credentials, None)
}

pub fn add_with_tracking(url: &str, name: &str, credentials: Option<(&str, &str)>, requested: Option<&Tracking>) -> Result<Collection> {
    let url = checked_add_url(url, name)?;
    let url = url.as_ref();
    if let Some((user, password)) = credentials {
        crate::auth::store(url, user, password)?;
    }
    clone(url, name, false, false, requested).inspect_err(|err| {
        if credentials.is_some() && err.is::<crate::auth::CredentialsRequired>() {
            crate::auth::forget(url); // wrong ones: don't keep them
        }
    })
}

/// Check a personal addition before requesting credentials or contacting Git. Return the
/// repository address to clone so a relative local path matches the same policy on later loads.
pub fn checked_add_url<'a>(url: &'a str, name: &str) -> Result<Cow<'a, str>> {
    check_name(name)?;
    let policy = crate::policy::load().collections;
    if !policy.users_can_add {
        bail!("your organization manages the collections; adding others isn't allowed");
    }
    if policy.preset.iter().any(|p| p.name == name) {
        bail!("'{name}' is the name of a collection your organization provides");
    }
    let path = Path::new(url);
    let url = if policy.has_user_url_restriction() && path.is_relative() && path.exists() {
        env::current_dir()?.join(path).to_string_lossy().into_owned().into()
    } else {
        Cow::Borrowed(url)
    };
    check_git_url(&policy, &url)?;
    Ok(url)
}

/// Add an organization's furnish address (`furnish.myorg.tld`, see `endpoint`): sign in if
/// needed (`show` gets the sign-in URL), then add each of its collections. Returns the new ones.
pub fn add_endpoint(origin: &str, show: impl Fn(&str)) -> Result<Vec<Collection>> {
    let description = crate::endpoint::discover(origin)?;
    if crate::endpoint::token(origin, &description).is_err() {
        crate::endpoint::login(origin, &description, show)?;
    }
    let presets = crate::policy::load().collections.preset;
    let mut shared = BTreeMap::new();
    let service = crate::service::enabled();
    for c in &description.collections {
        check_name(&c.name)?;
        if let Some(p) = presets.iter().find(|p| p.name == c.name) {
            anyhow::ensure!(p.url == c.url, "'{}' is the name of a collection your organization provides", c.name);
            if service {
                let dir = system_dir().join("collections").join(&c.name);
                let missing = !is_preset(&presets, &c.name, &dir);
                shared.insert(&c.name, (dir, missing));
            }
        }
    }
    // The service fetches the preset descriptions itself. Only credentials cross the
    // service boundary, even when the person initiates setup from an endpoint link.
    let problems = if shared.values().any(|(_, missing)| *missing) {
        crate::service::ensure_presets()?
    } else {
        Vec::new()
    };
    let mut added = Vec::new();
    let mut present = Vec::new();
    let result = (|| -> Result<()> {
        for c in &description.collections {
            if let Some((dir, missing)) = shared.get(&c.name) {
                if *missing {
                    anyhow::ensure!(is_preset(&presets, &c.name, dir), "could not set up shared collection '{}': {}", c.name,
                        if problems.is_empty() { "the service did not find this preset".into() } else { problems.join(", ") });
                    added.push(Collection { name: c.name.clone(), dir: dir.clone(), managed: true });
                }
                present.push(c.name.clone());
                continue;
            }
            let dir = collections_dir().join(&c.name);
            if dir.join(".git").exists() {
                match origin_url(&dir) {
                    Ok(url) if url == c.url => {
                        present.push(c.name.clone()); // already there
                        continue;
                    }
                    _ => bail!("a collection named '{}' already exists", c.name),
                }
            }
            let managed = presets.iter().any(|p| p.name == c.name);
            added.push(clone(&c.url, &c.name, managed, false, None)?);
            present.push(c.name.clone());
        }
        Ok(())
    })();
    // What it lists later on top of these is new (see `add_new`).
    let marked = crate::endpoint::mark_offered(origin, &present);
    result?;
    marked?;
    Ok(added)
}

/// What to do with a collection that an endpoint the person added lists, and that furnish hasn't
/// added for it before.
#[derive(Debug, PartialEq)]
enum Offer<'a> {
    Add(&'a crate::endpoint::Collection),
    /// The person has it already, from the same URL.
    Present(&'a str),
    /// Not added, for this reason.
    Refused(String),
}

/// The collections `listed` by an endpoint that aren't `offered` yet, and what to do with each,
/// by the rules for adding an endpoint (`add_endpoint`): a valid name, not a preset's name (a
/// preset with the same URL is left to `ensure_presets`), and not a collection of the person's
/// from another URL (`local_url`: the git URL of the person's collection of that name).
fn offers<'a>(
    listed: &'a [crate::endpoint::Collection],
    offered: &[String],
    presets: &[crate::policy::Preset],
    local_url: impl Fn(&str) -> Option<String>,
) -> Vec<Offer<'a>> {
    listed
        .iter()
        .filter(|c| !offered.contains(&c.name))
        .filter(|c| !presets.iter().any(|p| p.name == c.name && p.url == c.url))
        .map(|c| {
            if let Err(err) = check_name(&c.name) {
                return Offer::Refused(format!("{err:#}"));
            }
            if presets.iter().any(|p| p.name == c.name) {
                return Offer::Refused(format!("'{}' is the name of a collection your organization provides", c.name));
            }
            match local_url(&c.name) {
                Some(url) if url == c.url => Offer::Present(&c.name),
                Some(_) => Offer::Refused(format!("a collection named '{}' already exists", c.name)),
                None => Offer::Add(c),
            }
        })
        .collect()
}

/// Add the collections new in the description of an endpoint the person added: those it lists
/// that furnish didn't add for it before (see `endpoint::offered`). A collection the person
/// removed isn't added again, and one the endpoint stops listing stays. Returns the added
/// collections and problems as messages; a collection that couldn't be added is tried again next
/// time.
pub fn add_new(origin: &str, description: &crate::endpoint::Description) -> (Vec<Collection>, Vec<String>) {
    let policy = crate::policy::load();
    if let Err(err) = policy.check_endpoint(origin, &description.collections, check_git_url) {
        return (Vec::new(), vec![format!("{err:#}")]);
    }
    let presets = policy.collections.preset;
    let local_url = |name: &str| {
        let dir = collections_dir().join(name);
        dir.join(".git").exists().then(|| origin_url(&dir).ok()).flatten()
    };
    let host = crate::endpoint::url_host(origin);
    let (mut added, mut present, mut problems) = (Vec::new(), Vec::new(), Vec::new());
    for offer in offers(&description.collections, &crate::endpoint::offered(origin), &presets, local_url) {
        match offer {
            Offer::Add(c) => match clone(&c.url, &c.name, false, false, None) {
                Ok(collection) => {
                    present.push(c.name.clone());
                    added.push(collection);
                }
                Err(err) => problems.push(format!("{host}: couldn't add its new collection '{}': {err:#}", c.name)),
            },
            Offer::Present(name) => present.push(name.to_string()),
            Offer::Refused(why) => problems.push(format!("{host}: not adding its new collection: {why}")),
        }
    }
    if let Err(err) = crate::endpoint::mark_offered(origin, &present) {
        problems.push(format!("{host}: {err:#}"));
    }
    (added, problems)
}

/// Clone the admin's preset collections this user doesn't have yet. Returns problems as messages
/// (e.g. offline, or "sign in to …"), so a missing preset doesn't stop furnish.
pub fn ensure_presets() -> Vec<String> {
    let mut problems = Vec::new();
    // The organization's endpoints: their descriptions (and so their collections) may change.
    for e in crate::policy::load_native().endpoints {
        match crate::endpoint::origin(&e) {
            Some(origin) => {
                if let Err(err) = crate::endpoint::discover(&origin) {
                    problems.push(format!("{err:#}"));
                }
            }
            None => problems.push(format!("the admin policy names '{e}' as an endpoint, which isn't a furnish address")),
        }
    }
    let presets = crate::policy::load().collections.preset;
    if crate::service::enabled() {
        // The service sets them up; ask it only if one is missing or comes from elsewhere.
        if presets.iter().all(|p| is_preset(&presets, &p.name, &preset_dir().join(&p.name))) {
            return problems;
        }
        problems.extend(crate::service::ensure_presets().unwrap_or_else(|err| vec![format!("{err:#}")]));
        return problems;
    }
    for preset in &presets {
        let dir = collections_dir().join(&preset.name);
        if is_preset(&presets, &preset.name, &dir) {
            continue;
        }
        // A collection of that name from another URL (added by the user, or the policy changed
        // the URL): replaced by a fresh copy, never updated from the old URL.
        let replacing = dir.exists();
        match clone(&preset.url, &preset.name, true, true, None) {
            Ok(_) if replacing => eprintln!("note: replaced collection '{}' with your organization's from {}", preset.name, preset.url),
            Ok(_) => {}
            Err(err) if replacing => problems.push(format!(
                "collection '{}' doesn't come from {}, the address your organization gives; couldn't replace it: {err:#}",
                preset.name, preset.url
            )),
            Err(err) => problems.push(format!("couldn't set up collection '{}': {err:#}", preset.name)),
        }
    }
    problems
}

/// The name becomes a directory name: no separators, no `..`, no hidden dirs, nothing Windows
/// reserves (`CON`, `NUL`, `COM1`, …, also with an extension, or a trailing `.`).
pub(crate) fn check_name(name: &str) -> Result<()> {
    let valid = !name.is_empty()
        && !name.starts_with('.')
        && !name.ends_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    if !valid {
        bail!("invalid collection name '{name}': use letters, digits, '.', '_' and '-'");
    }
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].contains(&stem.as_str())
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        bail!("invalid collection name '{name}': Windows reserves it for a device");
    }
    Ok(())
}

/// Clone into `collections/<name>`. `replace`: a directory of that name (from another URL) is
/// replaced once the new copy is complete.
fn clone(url: &str, name: &str, managed: bool, replace: bool, requested: Option<&Tracking>) -> Result<Collection> {
    check_name(name)?;
    fs::create_dir_all(collections_dir().join(".partial"))?;
    let _lock = lock(&collections_dir(), name)?;
    let dir = collections_dir().join(name);
    if dir.exists() {
        if !replace {
            bail!("a collection named '{name}' already exists");
        }
        if origin_urls_match(&dir, |origin| origin == url) {
            return Ok(Collection { name: name.to_string(), dir, managed }); // another furnish just did
        }
    }
    let tracking = crate::settings::tracking_for(name, url, managed, requested)?;
    let target_ref = Target::resolve(None, url, &tracking)?;
    let settings = crate::settings::for_collection(name).settings;
    let depth = format!("--depth={}", settings.checkout_depth);
    let branch = format!("--branch={}", target_ref.short_name());
    let mut args = vec!["clone", "--quiet", "--no-checkout", "--single-branch", &branch];
    // A path on this computer is copied as it is, so there is nothing to download on demand; and
    // a filter would make git serve it like a remote (git-upload-pack), as whoever clones.
    if settings.blobs_on_demand && !Path::new(url).exists() {
        args.push("--filter=blob:none");
    }
    if settings.checkout_depth > 0 {
        args.push(&depth);
    }
    // Commits that left the history (see checkout_depth) are deleted at the next cleanup.
    args.extend(["--config=gc.reflogExpireUnreachable=now", "--config=gc.pruneExpire=now"]);
    // Set up in .partial/ and moved into place when complete: furnish may stop half way (the
    // service when it is replaced, say), and a copy without its checkout mustn't look finished.
    // Its own directory per attempt; under the lock, others of this name are left over.
    remove_partial(name);
    let staging = collections_dir().join(".partial").join(format!("{name}+{}", unique()));
    let target = staging.to_string_lossy().into_owned();
    args.extend(["--end-of-options", url, &target]);
    let collection = Collection { name: name.to_string(), dir: staging.clone(), managed };
    let set_up = || -> Result<()> {
        git(None, &args)?;
        target_ref.cloned(&collection.dir, settings.checkout_depth)?;
        collection.verify(&target_ref.commit)?;
        collection.prepare_checkout(&settings, &target_ref.commit, true)?;
        collection.checkout(&target_ref.commit)?;
        let backup = collections_dir().join(".partial").join(format!("{name}+{}", unique()));
        // A new personal collection can reuse the name of one that was removed.
        if !managed && !dir.exists() {
            crate::package_receipts::forget_collection(&data_dir(), name)?;
        }
        let replacing = dir.exists();
        if replacing {
            fs::rename(&dir, &backup)?;
        }
        if let Err(err) = fs::rename(&staging, &dir) {
            if replacing { fs::rename(&backup, &dir).context("restoring original collection after a failed replacement")?; }
            return Err(err.into());
        }
        if let Some(requested) = requested
            && let Err(err) = persist_tracking(name, Some(requested))
        {
            remove_repository(&dir);
            if replacing { fs::rename(&backup, &dir).context("restoring original collection after a settings error")?; }
            return Err(err);
        }
        if replacing { remove_repository(&backup); }
        remember_remote_head(&remote_heads_file(), name, &source_key(url, &tracking), &target_ref.commit);
        Ok(())
    };
    if let Err(err) = set_up() {
        remove_repository(&staging);
        return Err(err);
    }
    Ok(Collection { dir, ..collection })
}

/// Delete what earlier attempts to clone `name` left in .partial/ (call with the lock held).
fn remove_partial(name: &str) {
    let prefix = format!("{name}+");
    for entry in fs::read_dir(collections_dir().join(".partial")).into_iter().flatten().flatten() {
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            remove_repository(&entry.path());
        }
    }
}

/// A name no other attempt uses: process id, time and a counter.
fn unique() -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{}-{nanos}-{n}", std::process::id())
}

/// Hold the lock of collection `name` in `parent` (`.<name>.lock`): one clone, update or install
/// at a time per collection, across threads and processes. `None` where this user can't write
/// (the service's copies, which only the service changes).
fn lock(parent: &Path, name: &str) -> Result<Option<fs::File>> {
    let path = parent.join(format!(".{name}.lock"));
    match fs::OpenOptions::new().create(true).truncate(false).write(true).open(&path) {
        Ok(file) => {
            file.lock().with_context(|| format!("locking {}", path.display()))?;
            Ok(Some(file))
        }
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => Ok(None),
        Err(err) => Err(anyhow::Error::new(err).context(format!("locking {}", path.display()))),
    }
}

/// The newest commit each collection's server had when furnish last looked, with when that was
/// (`remote-heads.json` in the data folder). An update is available while it differs from the
/// commit checked out: looking never applies anything, and a commit that fails the checks (e.g. a
/// missing signature) stays available, not applied.
fn remote_heads_file() -> PathBuf {
    data_dir().join("remote-heads.json")
}

fn read_remote_heads(file: &Path) -> serde_json::Map<String, Value> {
    fs::read(file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn source_key(url: &str, tracking: &Tracking) -> String {
    cache_key(serde_json::json!([url, tracking]).to_string().as_bytes())
}

pub(crate) fn cache_key(input: &[u8]) -> String {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use sha2::{Digest, Sha256};
    // Avoid writing a credential-bearing remote URL into the status cache.
    URL_SAFE_NO_PAD.encode(Sha256::digest(input))
}

fn persist_tracking(name: &str, requested: Option<&Tracking>) -> Result<()> {
    crate::settings::set_for_collection(name, "tracking", serde_json::to_value(requested)?)
}

fn remember_remote_head(file: &Path, name: &str, source: &str, commit: &str) {
    edit_remote_heads(file, |heads| {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        heads.insert(name.into(), serde_json::json!({ "commit": commit, "source": source, "checkedAt": now }));
    });
}

/// Checks of different collections run concurrently. Serialize the read/modify/write and let
/// readers see the old or new complete cache. A cache-write failure only delays the indicator.
fn edit_remote_heads(file: &Path, edit: impl FnOnce(&mut serde_json::Map<String, Value>)) {
    let Some(dir) = file.parent() else { return };
    let Ok(Some(_lock)) = lock(dir, "remote-heads") else { return };
    let mut heads = read_remote_heads(file);
    edit(&mut heads);
    if let Ok(mut temp) = tempfile::NamedTempFile::new_in(dir)
        && serde_json::to_writer_pretty(temp.as_file_mut(), &heads).is_ok() {
        let _ = temp.persist(file);
    }
}

fn remote_head_differs(file: &Path, name: &str, source: &str, head: &str) -> bool {
    let heads = read_remote_heads(file);
    let Some(cached) = heads.get(name) else { return false };
    cached["source"].as_str() == Some(source) && cached["commit"].as_str().is_some_and(|c| c != head)
}

pub fn remove(name: &str) -> Result<()> {
    let collection = get(name)?;
    if collection.managed {
        bail!("'{name}' is provided by your organization and can't be removed");
    }
    let _lock = collection.lock()?;
    let url = collection.url().ok();
    let partial = collections_dir().join(".partial");
    fs::create_dir_all(&partial)?;
    let removed = partial.join(format!("{name}+{}", unique()));
    fs::rename(&collection.dir, &removed)?;
    if let Err(err) = crate::settings::forget_tracking(name) {
        fs::rename(&removed, &collection.dir).context("restoring collection after its tracking setting could not be cleared")?;
        return Err(err);
    }
    remove_repository(&removed);
    status_cache::forget(name);
    edit_remote_heads(&remote_heads_file(), |heads| { heads.remove(name); });
    if let Some(url) = url { crate::auth::forget(&url); }
    Ok(())
}

impl Collection {
    pub fn revision(&self) -> Result<String> {
        git(Some(&self.dir), &["rev-parse", "--short", "HEAD"])
    }

    pub fn url(&self) -> Result<String> {
        git(Some(&self.dir), &["remote", "get-url", "origin"])
    }

    /// The collection's own options (`collection.yaml`), resolved.
    pub fn options(&self) -> Result<Vec<crate::options::Resolved>> {
        Ok(crate::options::resolve_collection(&self.name, &crate::options::collection_declarations(&self.dir)?))
    }

    /// Required options nobody answered yet: its apps can't be installed until they are.
    pub fn unanswered(&self) -> Vec<crate::options::Resolved> {
        self.options().unwrap_or_default().into_iter().filter(|r| r.missing).collect()
    }

    /// The collection's translations (`locales/`), see `locale`.
    pub fn locales(&self) -> crate::locale::Locales {
        crate::locale::load_reporting(&self.dir)
    }

    /// The collection's avatar (`icon` in `collection.yaml`).
    pub fn icon(&self) -> Option<PathBuf> {
        self.file_in_repo(&crate::options::collection_icons(&self.dir).icon?)
    }

    /// The avatar for dark mode (`iconDark` in `collection.yaml`).
    pub fn icon_dark(&self) -> Option<PathBuf> {
        self.file_in_repo(&crate::options::collection_icons(&self.dir).icon_dark?)
    }

    /// Resolve the tracked remote commit without checking out files. Date-ordered tags may
    /// download missing tag/commit metadata; later checks reuse dates of immutable objects.
    /// Offline or failing: an error, and what was remembered stays.
    pub fn check_server(&self) -> Result<()> {
        let _lock = self.observation_lock()?;
        let url = origin_url(&self.dir)?;
        let tracking = crate::settings::tracking_for(&self.name, &url, self.managed, None)?;
        let target = self.resolve_target(&url, &tracking, true)?;
        remember_remote_head(&remote_heads_file(), &self.name, &source_key(&url, &tracking), &target.commit);
        Ok(())
    }

    /// Whether the last check of the current source found a different commit. A source can
    /// change when an endpoint, administrator or user changes the tracking preference.
    pub fn update_available(&self) -> bool {
        let Ok(url) = origin_url(&self.dir) else { return false };
        let Ok(tracking) = crate::settings::tracking_for(&self.name, &url, self.managed, None) else { return false };
        self.head().is_ok_and(|head| remote_head_differs(&remote_heads_file(), &self.name, &source_key(&url, &tracking), &head))
    }

    /// Apply the chosen target before persisting the user's choice. None resumes inherited tracking.
    pub fn set_tracking(&self, requested: Option<&Tracking>) -> Result<(String, String)> {
        let _lock = self.lock()?;
        let url = origin_url(&self.dir)?;
        let tracking = crate::settings::tracking_for_change(&self.name, &url, self.managed, requested)?;
        let old = self.head()?;
        let updated = self.update_target(&url, &tracking, false, None)?;
        if let Err(err) = persist_tracking(&self.name, requested) {
            self.checkout(&old).context("restoring the previous revision after a settings error")?;
            return Err(err);
        }
        if let Ok(head) = self.head() {
            remember_remote_head(&remote_heads_file(), &self.name, &source_key(&url, &tracking), &head);
        }
        Ok(updated)
    }

    /// Fast-forward to the latest upstream commit. Returns (old, new) revision.
    pub fn update(&self) -> Result<(String, String)> {
        self.update_reviewed(None)
    }

    /// Apply only when the current checkout and verified remote still match a comparison.
    pub fn update_reviewed(&self, reviewed: Option<&ReviewedRevisions>) -> Result<(String, String)> {
        let result = self.update_unreported(reviewed);
        let error = result.as_ref().err().map(|e| format!("{e:#}"));
        if let Some(message) = &error
            && !crate::service::in_service()
            && let Err(e) = crate::diagnostics::collection_update_failed(&data_dir(), &self.name, message)
        {
            eprintln!("warning: couldn't write the collection update log: {e:#}");
        }
        crate::reports::record(crate::reports::Event {
            event: "collectionUpdate",
            outcome: if result.is_ok() { "succeeded" } else { "failed" },
            collection: Some(self.name.clone()),
            version: result.as_ref().ok().map(|(_, new)| new.clone()),
            error,
            ..Default::default()
        });
        result
    }

    fn update_unreported(&self, reviewed: Option<&ReviewedRevisions>) -> Result<(String, String)> {
        if self.managed && crate::service::enabled() {
            let _poll = self.observation_lock()?;
            // The service resolves its own policy. Keep the client's remote observation so
            // update_available compares it with the checkout the service actually applied.
            return crate::service::update_reviewed(&self.name, reviewed);
        }
        let _lock = self.lock()?;
        let url = origin_url(&self.dir)?;
        let tracking = crate::settings::tracking_for(&self.name, &url, self.managed, None)?;
        self.update_target(&url, &tracking, true, reviewed)
    }

    /// Serialize observations with local updates. With the service, use a per-client lock
    /// for its own cache, since the shared repository belongs to the service.
    fn observation_lock(&self) -> Result<Option<fs::File>> {
        if self.managed && crate::service::enabled() {
            let dir = data_dir();
            fs::create_dir_all(&dir)?;
            return lock(&dir, &format!("remote-check.{}", self.name));
        }
        self.lock()
    }

    fn checkout(&self, commit: &str) -> Result<()> {
        git(Some(&self.dir), &["checkout", "--quiet", "--force", "--detach", "--no-recurse-submodules", commit, "--"])?;
        Ok(())
    }

    fn resolve_target(&self, url: &str, tracking: &Tracking, current_source: bool) -> Result<Target> {
        Target::resolve(Some(&self.dir), "origin", tracking).inspect_err(|err| {
            // A deleted branch or empty matching tag set invalidates an old notice. A failed
            // network request says nothing about the server's refs, so retain that observation.
            if current_source && err.is::<tracking::MissingRef>() {
                let source = source_key(url, tracking);
                edit_remote_heads(&remote_heads_file(), |heads| {
                    if heads.get(&self.name).is_some_and(|cached| cached["source"] == source) { heads.remove(&self.name); }
                });
            }
        })
    }

    fn verified_target(&self, url: &str, tracking: &Tracking, current_source: bool, depth: u32) -> Result<Target> {
        let target = self.resolve_target(url, tracking, current_source)?;
        target.fetch(&self.dir, depth)?;
        if current_source {
            remember_remote_head(&remote_heads_file(), &self.name, &source_key(url, tracking), &target.commit);
        }
        self.verify(&target.commit)?;
        self.require_descendant(&target)?;
        Ok(target)
    }

    fn update_target(&self, url: &str, tracking: &Tracking, current_source: bool, reviewed: Option<&ReviewedRevisions>) -> Result<(String, String)> {
        let old = self.head()?;
        let old_revision = self.revision()?;
        let settings = crate::settings::for_collection(&self.name).settings;
        let target = self.verified_target(url, tracking, current_source, settings.checkout_depth)?;
        if let Some(review) = reviewed {
            anyhow::ensure!(review.current == old && review.candidate == target.commit,
                "the collection changed since this comparison; preview the update again");
        }
        let checkout = self.prepare_checkout(&settings, &target.commit, false).and_then(|()| self.checkout(&target.commit));
        if let Err(err) = checkout {
            self.checkout(&old).context("restoring the previous revision after checkout failed")?;
            return Err(err);
        }
        Ok((old_revision, self.revision()?))
    }

    /// The collection's lock (see `lock`).
    fn lock(&self) -> Result<Option<fs::File>> {
        // The service locks its own clone when a per-user install delegates a rebuild.
        // Even a client with write access must not hold that same lock across the request.
        if self.managed && crate::service::enabled() {
            return Ok(None);
        }
        lock(self.dir.parent().unwrap_or(&self.dir), &self.name)
    }

    /// With trusted keys: `new` must follow the current commit. Otherwise whoever controls the
    /// server could move the collection back to an older signed commit, with its known problems.
    fn require_descendant(&self, target: &Target) -> Result<()> {
        let new = target.commit.as_str();
        if !self.managed || crate::policy::load().collections.trusted_keys.is_empty() {
            return Ok(());
        }
        let head = self.head()?;
        let follows = || git(Some(&self.dir), &["merge-base", "--is-ancestor", "--end-of-options", &head, new]).is_ok();
        if follows() {
            return Ok(());
        }
        // A shallow history may end before the current commit: fetch the rest and look again.
        if git(Some(&self.dir), &["rev-parse", "--is-shallow-repository"])? == "true" {
            target.fetch(&self.dir, 0)?;
            if follows() {
                return Ok(());
            }
        }
        bail!(
            "collection '{}': the new commit {} doesn't follow the current one {}; furnish doesn't go back to \
             earlier commits of collections with trusted keys (to start over from the server's, remove the \
             collection from the policy and add it again)",
            self.name,
            short(new),
            short(&head)
        )
    }

    /// Set up what checking out `rev` writes: git-lfs (so files it checks out are downloaded,
    /// not left as pointers), and with blobs_on_demand everything except installer files.
    fn prepare_checkout(&self, settings: &crate::settings::CollectionSettings, rev: &str, fresh: bool) -> Result<()> {
        let attributes = git(Some(&self.dir), &["cat-file", "-p", "--end-of-options", &format!("{rev}:.gitattributes")]).unwrap_or_default();
        if attributes.contains("filter=lfs") {
            git(Some(&self.dir), &["lfs", "version"]).map_err(|_| {
                anyhow::anyhow!("collection '{}' stores files with Git LFS, but git-lfs is not installed", self.name)
            })?;
            git(Some(&self.dir), &["lfs", "install", "--local"])?;
        }
        let sparse = self.dir.join(".git").join("info").join("sparse-checkout");
        let enabled = git(Some(&self.dir), &["config", "core.sparseCheckout"]).is_ok_and(|v| v == "true");
        if settings.blobs_on_demand && !enabled {
            // Installers already checked out stay: an installed app's status needs its file.
            let kept = if fresh { String::new() } else { git(Some(&self.dir), &[&["ls-files", "-z", "--"][..], &INSTALLERS].concat())? };
            let mut patterns = String::from("/*\n");
            for pattern in INSTALLERS {
                patterns += &format!("!{pattern}\n");
            }
            for file in kept.split('\0').filter(|f| !f.is_empty()) {
                patterns += &sparse_pattern(file)?;
            }
            fs::create_dir_all(sparse.parent().unwrap_or(&self.dir))?;
            fs::write(&sparse, patterns)?;
            for (key, value) in [
                ("core.sparseCheckout", "true"),
                ("core.sparseCheckoutCone", "false"),
                // Existing full copies: from now on, fetches leave out file contents too.
                ("remote.origin.promisor", "true"),
                ("remote.origin.partialCloneFilter", "blob:none"),
            ] {
                git(Some(&self.dir), &["config", key, value])?;
            }
        } else if !settings.blobs_on_demand && enabled {
            git(Some(&self.dir), &["config", "core.sparseCheckout", "false"])?;
            if !fresh {
                git(Some(&self.dir), &["read-tree", "-mu", "HEAD"])?;
            }
        }
        Ok(())
    }

    /// Check out files that blobs_on_demand left out (paths relative to the collection).
    fn check_out(&self, files: &[String]) -> Result<()> {
        let sparse = self.dir.join(".git").join("info").join("sparse-checkout");
        let mut patterns = fs::read_to_string(&sparse).unwrap_or_default();
        for file in files {
            let file = if cfg!(windows) { file.replace('\\', "/") } else { file.clone() };
            patterns += &sparse_pattern(&file)?;
        }
        fs::write(&sparse, patterns)?;
        git(Some(&self.dir), &["read-tree", "-mu", "HEAD"])?;
        for file in files {
            let path = self.dir.join(file);
            if !fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) {
                bail!("collection '{}' has no file {file}", self.name);
            }
            // What was checked out must be where furnish expects it: no symbolic links on the way.
            if !same_path(self.file_in_repo(file), self.file_in_repo_later(file)) {
                bail!("collection '{}': {file} is outside the collection", self.name);
            }
        }
        Ok(())
    }

    /// For preset collections, when the policy lists trusted keys: refuse a commit that isn't
    /// signed by one of them (SSH signatures). Whoever can push there controls what gets installed.
    fn verify(&self, rev: &str) -> Result<()> {
        let keys = crate::policy::load().collections.trusted_keys;
        if !self.managed || keys.is_empty() {
            return Ok(());
        }
        let refused = || format!("collection '{}': commit {} isn't signed by a key your organization trusts", self.name, short(rev));
        // git verifies a signature with the program for its kind: an OpenPGP signature would be
        // checked against the keyring of whoever runs furnish. Only SSH signatures count.
        let commit = git(Some(&self.dir), &["cat-file", "commit", "--end-of-options", rev]).with_context(refused)?;
        if !ssh_signed(&commit) {
            bail!("{} (furnish accepts SSH signatures only)", refused());
        }
        // A file of its own: users install per-user apps from the service's copy, which they can't change.
        let signers = env::temp_dir().join(format!("furnish-trusted-keys-{}", unique()));
        let lines: String = keys.iter().map(|k| format!("* namespaces=\"git\" {}\n", k.trim())).collect();
        fs::OpenOptions::new().write(true).create_new(true).open(&signers)?.write_all(lines.as_bytes())?;
        let _remove = Defer(|| {
            let _ = fs::remove_file(&signers);
        });
        let config = format!("gpg.ssh.allowedSignersFile={}", signers.display());
        #[rustfmt::skip]
        let args = [
            "-c", &config, "-c", "gpg.ssh.program=ssh-keygen", "-c", "gpg.ssh.revocationFile=",
            "-c", "gpg.program=false", "-c", "gpg.openpgp.program=false", "-c", "gpg.x509.program=false",
            "verify-commit", "--end-of-options", rev,
        ];
        git(Some(&self.dir), &args).with_context(refused)?;
        Ok(())
    }

    /// Load `apps/*.dsc.yaml`. Invalid files are reported on stderr and skipped,
    /// so one broken app doesn't hide the rest of the catalog.
    pub fn apps(&self) -> Vec<App> {
        let Ok(entries) = fs::read_dir(self.dir.join("apps")) else {
            return vec![];
        };
        let mut paths: Vec<_> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.to_string_lossy().ends_with(".dsc.yaml"))
            .collect();
        paths.sort();
        let held: Vec<_> = self.held_dirs().into_iter().map(|d| (crate::versions::all(&d), d)).collect();
        let allowed = allowed_resource_types();
        let scope = crate::package_receipts::scope(&self.name, self.managed);
        paths
            .iter()
            .filter_map(|path| match self.load_scoped_app(path, allowed.as_deref(), &scope) {
                Ok(app) => Some(self.with_hold(app, &held)),
                Err(err) => {
                    eprintln!("warning: skipping {}: {err:#}", path.display());
                    None
                }
            })
            .collect()
    }

    /// Where apps of this collection may be held at an earlier version: the user's data
    /// directory, and the service's for preset collections.
    fn held_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = vec![data_dir()];
        if self.managed && crate::service::enabled() && system_dir() != data_dir() {
            dirs.push(system_dir());
        }
        dirs
    }

    /// The app at the version it is held at, while the collection still has the version that failed.
    fn with_hold(&self, mut app: App, records: &[(BTreeMap<String, crate::versions::Record>, PathBuf)]) -> App {
        let key = app.key();
        let Some((held, dir)) = records.iter().find_map(|(r, d)| Some((r.get(&key)?.held.clone()?, d))) else {
            return app;
        };
        let relative = app.relative_path();
        let applies = match held.chosen {
            true => hold_applies(&held, None, crate::settings::for_collection(&self.name).settings.choose_version),
            false => hold_applies(&held, self.blob("HEAD", &relative).as_deref(), true),
        };
        if !applies {
            return app;
        }
        let load = || {
            let mut version = version_dir(dir, &self.name, &held.commit);
            if self.managed && let Err(err) = self.extract_scripts(&held.commit, &version) {
                // The service owns its snapshots. If an old one needs a script, a regular
                // user's status check can read the same revision from a private copy.
                let denied = err.chain().filter_map(|e| e.downcast_ref::<std::io::Error>()).any(|e| e.kind() == std::io::ErrorKind::PermissionDenied);
                if !denied || *dir != system_dir() || *dir == data_dir() { return Err(err); }
                version = version_dir(&data_dir(), &self.name, &held.commit);
                self.extract(&held.commit, &relative, &app.installed_values(), &version)?;
            }
            let file = version.join(&relative);
            Collection { name: self.name.clone(), dir: version, managed: self.managed }
                .load_scoped_app(&file, allowed_resource_types().as_deref(), &app.package_scope)
        };
        match load() {
            Ok(mut earlier) => {
                crate::required::carry(&app.doc, &mut earlier.doc);
                app.doc = earlier.doc;
                app.dependencies = earlier.dependencies;
                app.held = Some(held);
            }
            Err(err) => eprintln!("warning: {key}: the version it is held at: {err:#}"),
        }
        app
    }

    fn head(&self) -> Result<String> {
        git(Some(&self.dir), &["rev-parse", "HEAD"])
    }

    /// The id of the file's contents at `rev`, if the file exists there.
    fn blob(&self, rev: &str, relative: &str) -> Option<String> {
        let spec = format!("{rev}:{}", relative.replace('\\', "/"));
        git(Some(&self.dir), &["rev-parse", "--verify", "--quiet", "--end-of-options", &spec]).ok()
    }

    /// (commit, time, day) of `rev`.
    fn commit_info(&self, rev: &str) -> Result<(String, i64, String)> {
        let line = git(Some(&self.dir), &["log", "-1", "--format=%H %ct %cs", "--end-of-options", rev, "--"])?;
        parse_commit_line(&line).with_context(|| format!("unexpected git log output: {line}"))
    }

    /// The last `n` commits that changed the file, newest first. Not those that deleted it.
    fn changes(&self, relative: &str, n: usize) -> Vec<(String, i64, String)> {
        self.file_history(relative).into_iter().take(n).map(|(commit, time, day, _)| (commit, time, day)).collect()
    }

    /// Every commit in the history kept that changed the file (not those that deleted it),
    /// newest first: (commit, time, day, the file's blob id there).
    fn file_history(&self, relative: &str) -> Vec<(String, i64, String, String)> {
        // --raw: before git 2.48, --diff-filter without a diff output shows no commits at all.
        let args = ["log", "--no-renames", "--diff-filter=d", "--raw", "--no-abbrev", "--format=%H %ct %cs", "--", relative];
        let out = git(Some(&self.dir), &args).unwrap_or_default();
        let mut history: Vec<(String, i64, String, String)> = Vec::new();
        for line in out.lines() {
            // `:<old mode> <new mode> <old blob> <new blob> <status>\t<path>`
            if let Some(raw) = line.strip_prefix(':') {
                if let (Some(last), Some(blob)) = (history.last_mut(), raw.split_whitespace().nth(3)) {
                    last.3 = blob.to_string();
                }
            } else if let Some((commit, time, day)) = parse_commit_line(line) {
                history.push((commit, time, day, String::new()));
            }
        }
        // Merges show no raw diff.
        for entry in history.iter_mut().filter(|e| e.3.is_empty()) {
            entry.3 = self.blob(&entry.0, relative).unwrap_or_default();
        }
        history.retain(|e| !e.3.is_empty());
        history
    }

    /// The contents of blobs, downloading those a blobless copy lacks in one fetch.
    fn blobs(&self, ids: &[&str]) -> BTreeMap<String, Vec<u8>> {
        let missing: Vec<&str> = ids.iter().copied().filter(|id| !self.has_object(id)).collect();
        if !missing.is_empty() && git(Some(&self.dir), &["config", "remote.origin.promisor"]).is_ok_and(|v| v == "true") {
            // What git does for each missing object on its own, for all of them at once.
            let mut args = vec!["fetch", "--quiet", "--no-tags", "--no-write-fetch-head", "--recurse-submodules=no", "--filter=blob:none", "origin"];
            args.extend(&missing);
            if let Err(err) = git(Some(&self.dir), &[&["-c", "fetch.negotiationAlgorithm=noop"][..], &args].concat()) {
                eprintln!("warning: collection '{}': fetching earlier versions: {err:#}", self.name);
            }
        }
        ids.iter().filter_map(|id| Some((id.to_string(), git_bytes(Some(&self.dir), &["cat-file", "blob", id]).ok()?))).collect()
    }

    /// Whether the copy has the object, without downloading it.
    fn has_object(&self, id: &str) -> bool {
        let mut cmd = git_command();
        cmd.arg("-C").arg(&self.dir);
        if self.dir.starts_with(system_dir()) {
            cmd.args(["-c", "safe.directory=*"]);
        }
        cmd.env("GIT_NO_LAZY_FETCH", "1").args(["cat-file", "-e", id]);
        cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
    }

    /// Materialize a revision file once. Snapshots at one commit are shared by apps,
    /// so publish whole files atomically and leave files belonging to other apps in place.
    fn write_revision_file(&self, commit: &str, file: &str, into: &Path) -> Result<()> {
        self.write_revision_file_mode(commit, file, into, false)
    }

    fn write_revision_file_mode(&self, commit: &str, file: &str, into: &Path, executable: bool) -> Result<()> {
        #[cfg(not(unix))]
        let _ = executable;
        let root = into.canonicalize()?;
        if root.starts_with(self.dir.canonicalize()?) {
            bail!("a version snapshot must be outside the current checkout");
        }
        if !Path::new(file).components().all(|c| matches!(c, Component::Normal(n) if !n.eq_ignore_ascii_case(".git"))) {
            bail!("invalid path in version snapshot: {file}");
        }
        let target = root.join(file);
        let parent = target.parent().context("revision file needs a parent")?;
        let ancestor = parent.ancestors().find(|dir| dir.is_dir()).context("missing snapshot directory")?;
        if ancestor.canonicalize()? != ancestor { bail!("version snapshot traverses a symlink: {file}"); }
        fs::create_dir_all(parent)?;
        if parent.canonicalize()? != parent {
            bail!("version snapshot traverses a symlink: {file}");
        }
        if let Ok(info) = fs::symlink_metadata(&target) {
            if !info.is_file() { bail!("expected a regular file in the version snapshot: {file}"); }
            return Ok(());
        }
        // --filters returns checkout bytes, including Git LFS contents when configured.
        let spec = format!("{commit}:{}", file.replace('\\', "/"));
        let contents = git_bytes(Some(&self.dir), &["cat-file", "--filters", "--end-of-options", &spec])?;
        let builder = tempfile::Builder::new();
        #[cfg(unix)]
        let builder = {
            use std::os::unix::fs::PermissionsExt;
            let mut builder = builder;
            builder.permissions(fs::Permissions::from_mode(if executable { 0o777 } else { 0o666 })); // creation still respects the account's umask
            builder
        };
        let mut temp = builder.tempfile_in(parent)?;
        temp.write_all(&contents)?;
        match temp.persist_noclobber(&target) {
            Ok(_) => Ok(()),
            Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists && fs::symlink_metadata(&target)?.is_file() => Ok(()),
            Err(e) => Err(e.error.into()),
        }
    }

    fn write_revision_folder(&self, commit: &str, folder: &str, into: &Path) -> Result<()> {
        for (name, executable) in directory_tree(&self.dir, commit, folder)? {
            self.write_revision_file_mode(commit, &format!("{folder}/{name}"), into, executable)?;
        }
        Ok(())
    }

    /// The Git blob identifies the exact regular-file script selected by this commit.
    fn script_blob(&self, commit: &str, file: &str) -> Result<String> {
        let entry = git(Some(&self.dir), &["--literal-pathspecs", "ls-tree", "--end-of-options", commit, "--", file])?;
        if !(entry.starts_with("100644 blob ") || entry.starts_with("100755 blob ")) {
            bail!("package manager script '{file}' must be a regular file in this revision");
        }
        Ok(entry.split_whitespace().nth(2).context("missing script blob ID")?.into())
    }

    fn extract_scripts(&self, commit: &str, into: &Path) -> Result<()> {
        // A persisted hold supplies the commit, including when the cache predates script support.
        if !matches!(commit.len(), 40 | 64) || !commit.bytes().all(|c| c.is_ascii_hexdigit()) {
            bail!("invalid version commit '{commit}'");
        }
        if !into.join("collection.yaml").exists() && self.blob(commit, "collection.yaml").is_some() {
            self.write_revision_file(commit, "collection.yaml", into)?;
        }
        for def in crate::options::package_managers(into)?.values() {
            if !fs::symlink_metadata(into.join(&def.script)).is_ok_and(|m| m.is_file()) {
                self.script_blob(commit, &def.script)?;
                self.write_revision_file(commit, &def.script, into)?;
            }
        }
        Ok(())
    }

    /// Include declarations and their script contents in required-version decision memoization.
    fn manager_blobs(&self, commit: &str) -> Result<Vec<String>> {
        if !self.managed { return Ok(vec![]); }
        let Some(manifest) = self.blob(commit, "collection.yaml") else { return Ok(vec![]) };
        let bytes = git_bytes(Some(&self.dir), &["cat-file", "--filters", "--end-of-options", &format!("{commit}:collection.yaml")])?;
        let managers = crate::options::package_managers_from_yaml(&bytes)?;
        let mut blobs = vec![manifest];
        for def in managers.values() { blobs.push(self.script_blob(commit, &def.script)?); }
        Ok(blobs)
    }

    /// Write missing files from the chosen commit, including the collection's scripts and
    /// the installers the app uses with these values.
    fn extract(&self, commit: &str, relative: &str, values: &serde_json::Map<String, Value>, into: &Path) -> Result<()> {
        fs::create_dir_all(into)?;
        if self.managed {
            self.extract_scripts(commit, into)?;
        } else if !into.join("collection.yaml").exists() && self.blob(commit, "collection.yaml").is_some() {
            self.write_revision_file(commit, "collection.yaml", into)?;
        }
        self.write_revision_file(commit, relative, into)?;
        let earlier = Collection { name: self.name.clone(), dir: into.to_path_buf(), managed: self.managed };
        let app = earlier.load_app(&into.join(relative))?;
        for folder in app.directory_sources(values)? {
            self.write_revision_folder(commit, &folder, into)?;
        }
        for file in app.missing_installers(values) {
            self.write_revision_file(commit, &file, into)?;
        }
        Ok(())
    }

    /// Load `kits/*.yaml`. Kits naming unknown apps, or named like an app, are reported and skipped.
    pub fn kits(&self) -> Vec<Kit> {
        let Ok(entries) = fs::read_dir(self.dir.join("kits")) else {
            return vec![];
        };
        let mut paths: Vec<_> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "yaml"))
            .collect();
        paths.sort();
        let app_ids: Vec<String> = self.apps().into_iter().map(|a| a.id).collect();
        paths
            .iter()
            .filter_map(|path| match self.load_kit(path, &app_ids) {
                Ok(kit) => Some(kit),
                Err(err) => {
                    eprintln!("warning: skipping {}: {err:#}", path.display());
                    None
                }
            })
            .collect()
    }

    pub(crate) fn load_kit(&self, path: &Path, app_ids: &[String]) -> Result<Kit> {
        let file: KitFile = serde_yaml::from_str(&fs::read_to_string(path)?)?;
        let id = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
        if app_ids.contains(&id) {
            bail!("an app is also named '{id}'; kits and apps share names");
        }
        if file.items.is_empty() {
            bail!("a kit needs at least one item");
        }
        for item in &file.items {
            if !app_ids.contains(&item.app) {
                bail!("item '{}' is not an app in collection '{}'", item.app, self.name);
            }
        }
        Ok(Kit {
            id,
            collection: self.name.clone(),
            name: file.name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()),
            description: file.description.trim().to_string(),
            categories: file.categories,
            icon: file.icon.as_deref().and_then(|p| self.file_in_repo(p)),
            icon_dark: file.icon_dark.as_deref().and_then(|p| self.file_in_repo(p)),
            items: file.items,
            recommendation: file.recommended,
            collection_dir: self.dir.clone(),
        })
    }

    /// Images in Markdown descriptions are addressed from this collection's root.
    pub fn description_image(&self, relative: &str) -> Option<PathBuf> {
        if !Path::new(relative).components().all(|part| match part {
            Component::Normal(name) => !name.eq_ignore_ascii_case(".git"),
            Component::CurDir => true,
            _ => false,
        }) {
            return None;
        }
        let file = self.file_in_repo(relative)?;
        let root = plain_path(self.dir.canonicalize().ok()?);
        let inside = file.strip_prefix(root).ok()?;
        (file.is_file() && !inside.components().any(|part| part.as_os_str().eq_ignore_ascii_case(".git"))).then_some(file)
    }

    /// Resolve a repo-relative path; `None` if missing or pointing outside the repo.
    fn file_in_repo(&self, relative: &str) -> Option<PathBuf> {
        let path = self.dir.join(relative).canonicalize().ok()?;
        if !path.starts_with(self.dir.canonicalize().ok()?) {
            return None;
        }
        Some(plain_path(path))
    }

    /// Where a file of the repository that isn't checked out yet will be (see blobs_on_demand).
    /// Joined part by part onto the canonical directory: Windows installers such as msiexec reject
    /// paths with a `/`.
    fn file_in_repo_later(&self, relative: &str) -> Option<PathBuf> {
        let parts = Path::new(relative).components();
        if !parts.clone().all(|c| matches!(c, Component::Normal(_))) {
            return None;
        }
        Some(parts.fold(plain_path(self.dir.canonicalize().ok()?), |p, c| p.join(c)))
    }

    pub(crate) fn load_app(&self, path: &Path) -> Result<App> {
        self.load_app_allowing(path, allowed_resource_types().as_deref())
    }

    /// Load an app whose resources are of furnish's types or, with `allowed` (the admin policy's
    /// `allowedResourceTypes`), only of those.
    pub(crate) fn load_app_allowing(&self, path: &Path, allowed: Option<&[String]>) -> Result<App> {
        let scope = crate::package_receipts::scope(&self.name, self.managed);
        self.load_scoped_app(path, allowed, &scope)
    }

    fn load_scoped_app(&self, path: &Path, allowed: Option<&[String]>, scope: &str) -> Result<App> {
        let yaml: serde_yaml::Value = serde_yaml::from_str(&fs::read_to_string(path)?)?;
        let mut doc: Value = serde_json::to_value(yaml)?;
        anyhow::ensure!(doc.is_object(), "the app document must be an object");
        for (field, value) in [("metadata", &doc["metadata"]), ("metadata.furnish", &doc["metadata"]["furnish"])] {
            anyhow::ensure!(value.is_null() || value.is_object(), "{field} must be an object");
        }
        crate::script_managers::attach(self, &mut doc)?;
        if doc["variables"].get(crate::conditions::HOST).is_some() {
            bail!("DSC variable '{}' is supplied by furnish and cannot be redeclared", crate::conditions::HOST);
        }
        if doc.get("$schema").is_none() {
            // DSC requires it; don't make every app author repeat it.
            doc["$schema"] = DSC_SCHEMA.into();
        }
        let id = path.file_name().unwrap_or_default().to_string_lossy().trim_end_matches(".dsc.yaml").to_string();
        self.check_resources(&mut doc["resources"], allowed, &id)?;
        crate::resources::check_modes(&doc["resources"])?;
        crate::required::validate(&doc["resources"])?;
        // Removal steps are resources too: the same policy and installer checks.
        if let Some(steps) = doc.pointer_mut("/metadata/furnish/uninstall") {
            if !steps.as_array().is_some_and(|s| !s.is_empty() && s.iter().all(Value::is_object)) {
                bail!("metadata.furnish.uninstall: a list of DSC resources, like resources");
            }
            self.check_resources(steps, allowed, &id).context("metadata.furnish.uninstall")?;
        }
        crate::nixos_apps::stamp_packages(&mut doc, &format!("{}/{id}", self.name), scope)?;
        let meta = &doc["metadata"]["furnish"];
        if meta.get("requiresNewSession").is_some_and(|v| !v.is_boolean()) {
            bail!("metadata.furnish.requiresNewSession must be true or false");
        }
        Ok(App {
            package_scope: scope.into(),
            id,
            collection: self.name.clone(),
            name: meta["name"].as_str().map(|n| n.trim().to_string()).filter(|n| !n.is_empty()),
            description: meta["description"].as_str().unwrap_or_default().trim().to_string(),
            categories: meta["categories"]
                .as_array()
                .map(|c| c.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default(),
            icon: meta["icon"].as_str().and_then(|p| self.file_in_repo(p)),
            icon_dark: meta["iconDark"].as_str().and_then(|p| self.file_in_repo(p)),
            path: path.to_path_buf(),
            collection_dir: self.dir.clone(),
            managed: self.managed,
            dependencies: match meta.get("dependencies") {
                Some(deps) => serde_json::from_value(deps.clone()).context("metadata.furnish.dependencies")?,
                None => Vec::new(),
            },
            doc,
            held: None,
        })
    }

    /// Check the resources of a document, and those nested in groups and adapters
    /// (`properties.resources`): types the policy allows, installer and source files in the
    /// collection, and the properties of the configuration resources. `Furnish/Block` gets the
    /// app's name for its markers.
    fn check_resources(&self, resources: &mut Value, allowed: Option<&[String]>, app: &str) -> Result<()> {
        for resource in resources.as_array_mut().into_iter().flatten() {
            let name = resource["name"].as_str().unwrap_or("?").to_string();
            let kind = resource["type"].as_str().unwrap_or_default().to_string();
            // DSC matches types ignoring case. The checks below must see every Furnish/Installer.
            let canonical = RESOURCE_TYPES.iter().find(|t| t.eq_ignore_ascii_case(&kind));
            if let Some(canonical) = canonical {
                resource["type"] = (*canonical).into();
            } else if let Some(allowed) = allowed
                && !allowed.iter().any(|t| t.eq_ignore_ascii_case(&kind))
            {
                bail!("resource '{name}': type '{kind}' isn't allowed by your organization (allowed: {}, {})", RESOURCE_TYPES.join(", "), allowed.join(", "));
            }
            if resource["properties"]["resources"].is_array() {
                self.check_resources(&mut resource["properties"]["resources"], allowed, app)?;
            }
            if canonical == Some(&PACKAGE_TYPE) {
                flake::attach(&self.dir, resource).with_context(|| format!("resource '{name}'"))?;
            }
            if canonical == Some(&BLOCK_TYPE) {
                resource["properties"]["app"] = format!("{}/{app}", self.name).into();
            }
            let (key, what) = match canonical {
                Some(&INSTALLER_TYPE) => ("file", "installer"),
                Some(&FILE_TYPE | &COMMAND_TYPE | &DIRECTORY_TYPE) => ("source", "source"),
                Some(kind) => {
                    crate::resources::check(kind, &resource["properties"]).with_context(|| format!("resource '{name}'"))?;
                    continue;
                }
                None => continue,
            };
            // Windows passes properties as environment variables, whose names ignore case.
            let properties = &resource["properties"];
            let source_keys: &[&str] = if canonical == Some(&INSTALLER_TYPE) { &["file", "url", "sha256"] } else { &["source"] };
            for actual in properties.as_object().into_iter().flat_map(|p| p.keys()) {
                if let Some(expected) = source_keys.iter().find(|k| k.eq_ignore_ascii_case(actual) && **k != actual) {
                    bail!("{what} property '{actual}': did you mean '{expected}'?");
                }
            }
            if canonical == Some(&INSTALLER_TYPE) {
                match (properties.get("file"), properties.get("url"), properties.get("sha256")) {
                    (Some(Value::String(_)), None, None) => {}
                    (None, Some(Value::String(url)), Some(Value::String(pin))) => {
                        crate::installer_download::validate(url, pin).with_context(|| format!("resource '{name}'"))?;
                        continue;
                    }
                    _ => bail!("resource '{name}': Furnish/Installer needs either file or both url and sha256"),
                }
            }
            // Installer and source files are given relative to the collection; resources need absolute paths.
            if let Some(file) = resource["properties"][key].as_str() {
                if file.chars().any(char::is_control) {
                    bail!("{what} '{}' has control characters in its name", file.escape_debug());
                }
                if canonical == Some(&DIRECTORY_TYPE) {
                    let relative = Path::new(file);
                    anyhow::ensure!(!file.is_empty() && relative.components().all(|c| matches!(c, Component::Normal(n) if !n.eq_ignore_ascii_case(".git"))),
                        "source '{file}' must name a folder within the collection");
                }
                let outside = || format!("{what} '{file}' is outside the collection");
                let resolved = match fs::symlink_metadata(self.dir.join(file)) {
                    Ok(_) => self.file_in_repo(file).with_context(outside)?,
                    // Not checked out yet (blobs_on_demand): installing the app checks it out.
                    Err(_) => self.file_in_repo_later(file).with_context(outside)?,
                };
                if canonical == Some(&DIRECTORY_TYPE) {
                    let lexical = self.file_in_repo_later(file).with_context(outside)?;
                    anyhow::ensure!(same_path(Some(lexical), Some(resolved.clone())), "directory source '{file}' traverses a link");
                    // The trusted collection root also bounds source reads when a sparse file is missing.
                    resource["metadata"]["furnishDirectoryRoot"] = plain_path(self.dir.canonicalize()?).to_string_lossy().into_owned().into();
                    resource["metadata"]["furnishDirectoryOwner"] = format!("{}/{app}", self.name).into();
                    if let Some(props) = resource["properties"].as_object_mut() { props.remove("_metadata"); }
                }
                resource["properties"][key] = resolved.to_string_lossy().into_owned().into();
            } else if !resource["properties"][key].is_null() {
                bail!("{what} file must be a path in the collection");
            }
            if matches!(canonical, Some(&FILE_TYPE | &COMMAND_TYPE | &DIRECTORY_TYPE)) {
                crate::resources::check(&kind, &resource["properties"]).with_context(|| format!("resource '{name}'"))?;
            }
        }
        Ok(())
    }
}

/// The resource types the admin policy allows apps to use besides furnish's own; `None`: any.
pub(crate) fn allowed_resource_types() -> Option<Vec<String>> {
    crate::policy::load_native().allowed_resource_types
}

/// `App::outside_service` without the computer: `service`, the policy routes preset collections
/// through the furnish service (`service::enabled`); `managed`, the app's collection is preset.
fn outside_service(
    in_service: bool,
    service: bool,
    managed: bool,
    collection: &str,
    app: &str,
    per_user: impl FnOnce() -> bool,
) -> Option<crate::nixos_apps::OutsideService> {
    use crate::nixos_apps::OutsideService;
    match () {
        _ if in_service => None,
        _ if !service => Some(OutsideService::Off),
        _ if !managed => Some(OutsideService::NotPreset { collection: collection.to_string() }),
        _ if per_user() => Some(OutsideService::PerUser { app: app.to_string() }),
        _ => None,
    }
}

/// The resources furnish ships (see `engine::init`). Apps may use other DSC resources too.
pub const RESOURCE_TYPES: [&str; 9] = [
    PACKAGE_TYPE,
    INSTALLER_TYPE,
    "Furnish/NixOS",
    DIRECTORY_TYPE,
    FILE_TYPE,
    BLOCK_TYPE,
    "Furnish/Line",
    "Furnish/Value",
    COMMAND_TYPE,
];
pub(crate) const DIRECTORY_TYPE: &str = "Furnish/Directory";
const COMMAND_TYPE: &str = "Furnish/Command";
const FILE_TYPE: &str = "Furnish/File";
const BLOCK_TYPE: &str = "Furnish/Block";
pub const INSTALLER_TYPE: &str = "Furnish/Installer";
pub const PACKAGE_TYPE: &str = "Furnish/Package";

/// Whether two resolved paths are the same (Windows: ignoring case, like its file systems).
fn same_path(a: Option<PathBuf>, b: Option<PathBuf>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) if cfg!(windows) => a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase(),
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// A sparse-checkout line for exactly this file (gitignore syntax: wildcards and `\\` escaped).
fn sparse_pattern(file: &str) -> Result<String> {
    if file.chars().any(char::is_control) {
        bail!("'{}' has control characters in its name", file.escape_debug());
    }
    let mut pattern = String::from("/");
    for c in file.chars() {
        if "*?[]\\!# ".contains(c) {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    Ok(pattern + "\n")
}

/// Whether a commit (`git cat-file commit` output) carries SSH signatures, and only those.
fn ssh_signed(commit: &str) -> bool {
    let headers = commit.split("\n\n").next().unwrap_or_default();
    let signatures: Vec<&str> = headers.lines().filter(|l| l.starts_with("gpgsig")).collect();
    !signatures.is_empty() && signatures.iter().all(|l| l.split_once(' ').is_some_and(|(_, v)| v == "-----BEGIN SSH SIGNATURE-----"))
}

/// Runs its closure when dropped.
struct Defer<F: FnMut()>(F);

impl<F: FnMut()> Drop for Defer<F> {
    fn drop(&mut self) {
        (self.0)();
    }
}

fn short(commit: &str) -> &str {
    commit.get(..10).unwrap_or(commit)
}

/// Normal Windows paths for tools such as Windows PowerShell and msiexec.
/// Preserve the network root when converting a verbatim UNC path.
pub(crate) fn plain_path(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    if let Some(text) = path.to_str() {
        if let Some(share) = text.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{share}").into();
        }
        if let Some(drive) = text.strip_prefix(r"\\?\")
            && matches!(Path::new(drive).components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), std::path::Prefix::Disk(_)))
        {
            return drive.into();
        }
    }
    path
}

/// Delete a copy git left behind.
fn remove_repository(dir: &Path) {
    if let Err(err) = fs::remove_dir_all(dir)
        && dir.exists()
    {
        eprintln!("warning: couldn't remove {}: {err}", dir.display());
    }
}

fn parse_commit_line(line: &str) -> Option<(String, i64, String)> {
    let mut parts = line.split(' ');
    Some((parts.next()?.to_string(), parts.next()?.parse().ok()?, parts.next()?.to_string()))
}

/// Whether an app stays at the version it is held at. A version the person chose: while choosing
/// is on (`choose_version`); with it turned off, the collection's version applies. A fallback:
/// while the collection's file for the app (`head_blob`) is still the one that failed; once the
/// collection changes the app, the next install tries that version.
fn hold_applies(held: &crate::versions::Held, head_blob: Option<&str>, choose_version: bool) -> bool {
    if held.chosen { choose_version } else { head_blob == Some(held.failed.as_str()) }
}

/// Whether a version from `time` is more than `days` older than the current one from `newest`
/// (`fallbackMaxAgeDays`; 0: any age).
fn older_than(days: u32, time: i64, newest: i64) -> bool {
    days > 0 && newest - time > i64::from(days) * 86_400
}

/// The versions an app's file lists as acceptable: those of its first resource with `versions`.
fn stated_versions(file: &Value) -> Vec<crate::required::Entry> {
    let mut resources = file["resources"].as_array().into_iter().flatten();
    resources.find_map(|r| crate::required::parse(r["properties"].get(crate::required::PROPERTY)?).ok()).unwrap_or_default()
}

/// Where the files of an earlier version of a collection's apps are kept.
fn version_dir(state_dir: &Path, collection: &str, commit: &str) -> PathBuf {
    state_dir.join("versions").join(collection).join(commit)
}

impl App {
    pub fn key(&self) -> String {
        format!("{}/{}", self.collection, self.id)
    }

    /// The app's options, resolved (see `options`).
    pub fn options(&self) -> Vec<crate::options::Resolved> {
        let shared = crate::options::collection_declarations(&self.collection_dir).unwrap_or_else(|err| {
            eprintln!("warning: ignoring collection.yaml of {}: {err:#}", self.collection);
            Default::default()
        });
        crate::options::resolve_app(&self.collection, &self.id, &self.doc, &shared)
    }

    pub fn current_values(&self) -> serde_json::Map<String, Value> {
        crate::options::values(&self.options())
    }

    /// Recommended for this user: `metadata.furnish.recommended`, `true` or a DSC expression over
    /// the collection's and the app's options, e.g. `[equals(parameters('role'), 'developer')]`.
    pub fn recommended(&self) -> bool {
        recommended(&self.doc["metadata"]["furnish"]["recommended"], &self.collection, &self.collection_dir, Some(self))
    }

    /// Required collection options nobody answered yet (names); installing waits for them.
    pub fn unanswered(&self) -> Vec<String> {
        let shared = crate::options::collection_declarations(&self.collection_dir).unwrap_or_default();
        crate::options::resolve_collection(&self.collection, &shared).into_iter().filter(|r| r.missing).map(|r| r.name).collect()
    }

    fn require_answers(&self) -> Result<()> {
        let unanswered = self.unanswered();
        if let Some(name) = unanswered.first() {
            bail!(
                "{} first needs your answer to its question{} ({}): furnish option set {} {name} <value>",
                self.collection,
                if unanswered.len() > 1 { "s" } else { "" },
                unanswered.join(", "),
                self.collection
            );
        }
        Ok(())
    }

    /// Run in the signed-in user's account when the app declares perUser (optionally
    /// through a DSC expression) or an active step selects a per-user package manager.
    pub fn per_user(&self, values: &serde_json::Map<String, Value>) -> bool {
        let mut profiles = vec![data_dir()];
        if self.managed && crate::service::enabled() {
            profiles.push(system_dir());
        }
        self.per_user_in(values, &profiles)
    }

    fn per_user_in(&self, values: &serde_json::Map<String, Value>, profiles: &[PathBuf]) -> bool {
        let declared = match &self.doc["metadata"]["furnish"]["perUser"] {
            Value::Bool(b) => *b,
            Value::String(expr) => crate::conditions::holds_for(&self.doc, expr, values).unwrap_or_else(|err| {
                eprintln!("warning: {}: perUser: {err:#}", self.id);
                false
            }),
            _ => false,
        };
        declared || crate::package_names::requires_user_in(&self.doc, values, profiles)
    }

    /// Installed through the furnish service with these values.
    fn remote(&self, values: &serde_json::Map<String, Value>) -> bool {
        self.managed && crate::service::enabled() && !self.per_user(values)
    }

    /// Data directory of whoever installs with these values (the user, or the furnish service),
    /// which records the option values it was installed with.
    fn state_dir(&self, values: &serde_json::Map<String, Value>) -> PathBuf {
        // Inside the service, data_dir() is the system directory too.
        if self.remote(values) { system_dir() } else { data_dir() }
    }

    /// The values it was installed with: the user's record, else the service's.
    pub fn recorded_values(&self) -> Option<serde_json::Map<String, Value>> {
        crate::options::installed_with(&data_dir(), &self.key()).or_else(|| {
            (self.managed && crate::service::enabled()).then(|| crate::options::installed_with(&system_dir(), &self.key())).flatten()
        })
    }

    /// Values for status, updates and uninstall: those it was installed with, else the current ones.
    pub fn installed_values(&self) -> serde_json::Map<String, Value> {
        self.recorded_values().unwrap_or_else(|| self.current_values())
    }

    /// Installed with option values that differ from the current ones (a reinstall applies them).
    pub fn options_differ(&self) -> bool {
        let Some(installed) = self.recorded_values() else { return false };
        !crate::options::changed_values(&self.options(), Some(&installed)).is_empty()
    }

    pub fn doc_for(&self, values: &serde_json::Map<String, Value>) -> Value {
        let mut doc = crate::options::apply(&self.doc, values);
        // A client checking a service-installed app should query the profile the service
        // would change. The backend accepts this hint only for read-only operations.
        if self.remote(values) {
            doc["metadata"]["furnish"][crate::nixos_apps::SERVICE_PROFILE] = true.into();
        } else if let Some(meta) = doc["metadata"]["furnish"].as_object_mut() {
            meta.remove(crate::nixos_apps::SERVICE_PROFILE);
        }
        doc
    }

    /// `doc_for`, to run in this process: it says why the furnish service doesn't run it, for
    /// Furnish/NixOS (`nixos_apps::OutsideService`).
    fn doc_here(&self, values: &serde_json::Map<String, Value>) -> Value {
        let mut doc = self.doc_for(values);
        if let Some(furnish) = doc["metadata"]["furnish"].as_object_mut() {
            furnish.remove(crate::nixos_apps::OUTSIDE_SERVICE_KEY);
            if let Some(why) = self.outside_service(values) {
                let mut outside = serde_json::to_value(&why).unwrap_or_default();
                if matches!(why, crate::nixos_apps::OutsideService::PerUser { .. }) {
                    outside["app"] = self.key().into();
                    outside["values"] = Value::Object(values.clone());
                }
                furnish.insert(crate::nixos_apps::OUTSIDE_SERVICE_KEY.into(), outside);
            }
        }
        doc
    }

    /// Why this process installs the app with these values and not the furnish service (`None`:
    /// this is the service, or the service installs it).
    fn outside_service(&self, values: &serde_json::Map<String, Value>) -> Option<crate::nixos_apps::OutsideService> {
        outside_service(crate::service::in_service(), crate::service::enabled(), self.managed, &self.collection, &self.id, || self.per_user(values))
    }

    /// The service validates a single NixOS step against its current, verified preset checkout.
    /// A checkout held by the user may delegate only a step the current preset still authorizes.
    pub(crate) fn apply_nixos_step(&self, request: &crate::nixos_apps::ServiceStep) -> Result<()> {
        let collection = self.collection_handle();
        let _lock = collection.lock()?;
        collection.verify(&collection.head()?)?;
        let current = self.current()?;
        let values = crate::options::accept(&current.options(), &request.values)?;
        if !current.managed || !current.per_user(&values) {
            bail!("{} is not a preset per-user app with these options", current.key());
        }
        let doc = current.doc_here(&values);
        crate::engine::apply_nixos_step(&doc, request)
    }

    /// The resources that apply on this computer with the current option values, as the engine
    /// filters them (see `engine::applies_here`).
    pub fn resources_here(&self) -> Vec<Value> {
        let doc = self.doc_for(&self.current_values());
        let resources = doc["resources"].as_array().cloned().unwrap_or_default();
        resources.into_iter().filter(|r| crate::engine::applies_here(r, &doc)).collect()
    }

    /// Dependencies whose `condition` holds for these option values.
    pub fn dependencies_for(&self, values: &serde_json::Map<String, Value>) -> Vec<Item> {
        self.dependencies
            .iter()
            .filter(|d| match &d.condition {
                None => true,
                Some(c) => crate::conditions::holds_for(&self.doc, c, values).unwrap_or_else(|err| {
                    eprintln!("warning: {}: dependency {} condition: {err:#}", self.id, d.app);
                    false
                }),
            })
            .cloned()
            .collect()
    }

    pub fn check(&self) -> Result<crate::engine::Check> {
        let mut check = crate::engine::check(&self.doc_for(&self.installed_values()))?;
        // Held at an earlier version that offers nothing acceptable: the current version may.
        if self.held.is_some() && check.required.as_ref().is_some_and(|r| r.target.is_none()) {
            check = self.current()?.check()?;
        }
        crate::reports::requirement(self, check.required.as_ref());
        Ok(check)
    }

    /// Before an explicit uninstall, recover the package identity when it comes
    /// from a downloaded installer. An empty cache alone cannot prove the app is absent.
    pub fn check_for_uninstall(&self) -> Result<crate::engine::Check> {
        let doc = self.doc_for(&self.installed_values());
        for resource in doc["resources"].as_array().into_iter().flatten() {
            if resource["type"] == INSTALLER_TYPE && crate::engine::applies_here(resource, &doc) {
                crate::installer_download::prepare_removal_identity(&resource["properties"])?;
            }
        }
        self.check()
    }

    pub fn status(&self) -> Result<crate::engine::Status> {
        Ok(self.check()?.status)
    }

    pub fn plan(&self, uninstall: bool) -> Result<Vec<crate::engine::Step>> {
        let doc = self.doc_for(&self.installed_values());
        if uninstall {
            self.require_removable(&doc)?;
        }
        crate::engine::plan(&doc, uninstall)
    }

    /// Why furnish can't uninstall the app here (see `engine::unremovable`); `None` if it can.
    pub fn unremovable(&self) -> Result<Option<String>> {
        crate::engine::unremovable(&self.doc_for(&self.installed_values()))
    }

    fn require_removable(&self, doc: &Value) -> Result<()> {
        if let Some(what) = crate::engine::unremovable(doc)? {
            bail!(
                "furnish can't uninstall {}: {what} can't remove what it set up, and the app has no removal steps \
                 (metadata.furnish.uninstall). Ask whoever maintains the collection '{}' to add them",
                self.id,
                self.collection
            );
        }
        Ok(())
    }

    /// Uninstall here with these values: the removal steps, or the resources set to `_exist: false`.
    /// Then the app must no longer test as installed.
    fn remove_with(&self, values: &serde_json::Map<String, Value>, report: impl FnMut(crate::engine::Progress)) -> Result<()> {
        let doc = self.doc_here(values);
        self.require_removable(&doc)?;
        self.check_out_installers(values)?;
        crate::engine::run(&doc, true, report)?;
        use crate::engine::Status;
        if matches!(crate::engine::status(&doc)?, Status::Installed | Status::NeedsUpdate) {
            let what = if crate::engine::removal_steps(&doc).is_some() { "its removal steps ran" } else { "uninstalling finished" };
            return Err(logged(format!("{} is still installed: {what}, but its resources still find it on this computer", self.id)));
        }
        Ok(())
    }

    /// After installing: the app must test as installed. A step can report success without
    /// having done its job (a package manager that exits 0 on a failure, a script that ignores an
    /// error), so furnish doesn't trust the steps alone. The test is the status check furnish runs
    /// anyway to show the app, which is cheap next to the install.
    fn require_installed(&self, doc: &Value) -> Result<()> {
        let differing = crate::engine::differing(doc).with_context(|| format!("checking {} after the install", self.id))?;
        if differing.is_empty() {
            return Ok(());
        }
        let resources = match differing.as_slice() {
            [one] => format!("the resource {one} still differs"),
            more => format!("the resources {} still differ", more.join(", ")),
        };
        Err(logged(format!("{} isn't installed after the install: {resources}", self.id)))
    }

    /// Install or update, with the values it was installed with (else the current ones), and
    /// remember them.
    pub fn install(&self, report: impl FnMut(crate::engine::Progress)) -> Result<()> {
        self.install_version(Which::Shown, report)
    }

    /// Install the current version, the previous one (see `previous_version`) or a chosen one
    /// (see `versions`). Reported.
    pub fn install_version(&self, which: Which, report: impl FnMut(crate::engine::Progress)) -> Result<()> {
        let event = match which {
            Which::Previous => "fallback",
            _ if self.recorded_values().is_some() => "update",
            _ => "install",
        };
        let version = self.operation_revision(&which);
        crate::reports::operation(event, self, version, || self.install_values(self.installed_values(), which, report))
    }

    /// The target revision at dispatch, including pinned versions and explicit fallbacks.
    pub fn operation_revision(&self, which: &Which) -> Option<String> {
        match which {
            Which::Previous => self.previous_version().ok().map(|v| v.commit),
            Which::Version(commit) => Some(commit.clone()),
            Which::Shown if self.held.is_some() => self.held.as_ref().map(|h| h.commit.clone()),
            _ => self.collection_handle().head().ok(),
        }
    }

    /// Install with these values: through the furnish service, or here.
    fn install_values(&self, values: serde_json::Map<String, Value>, which: Which, report: impl FnMut(crate::engine::Progress)) -> Result<()> {
        self.require_answers()?;
        if self.remote(&values) {
            let install_into = crate::nixos_apps::own_install_into();
            return crate::service::run(&crate::service::Request::Install { app: self.key(), values, which, install_into }, report);
        }
        self.install_here(&values, which, report)
    }

    pub fn install_here(&self, values: &serde_json::Map<String, Value>, which: Which, report: impl FnMut(crate::engine::Progress)) -> Result<()> {
        let collection = self.collection_handle();
        let _lock = collection.lock()?;
        // The checkout may have changed since furnish verified it: only install a signed one.
        collection.verify(&collection.head()?)?;
        let result = match which {
            // Held at an earlier version that offers nothing acceptable: a required update
            // installs the collection's current version (see `check`).
            Which::Shown if self.held.is_some() && self.held_offers_nothing_acceptable(values)? => self.current()?.install_with(values, report),
            Which::Shown => self.install_with(values, report),
            Which::Current => self.current()?.install_with(values, report),
            Which::Previous => return self.install_previous_with(values, report),
            Which::Version(commit) => return self.install_chosen_with(&commit, values, report),
        };
        // Remembered, so the app may go back to an earlier version (see install_previous_with).
        if let Err(err) = &result
            && !err.is::<crate::engine::Cancelled>() && !err.is::<crate::engine::WaitingForClose>()
            && let Some(blob) = collection.blob("HEAD", &self.relative_path())
            && let Err(record) = crate::versions::failed(&self.state_dir(values), &self.key(), &blob)
        {
            eprintln!("warning: {}: couldn't record the failure: {record:#}", self.key());
        }
        result
    }

    pub fn install_with(&self, values: &serde_json::Map<String, Value>, report: impl FnMut(crate::engine::Progress)) -> Result<()> {
        self.check_out_installers(values)?;
        let doc = self.doc_here(values);
        crate::engine::run(&doc, false, report)?;
        self.require_installed(&doc)?;
        let state = self.state_dir(values);
        crate::options::remember(&state, &self.key(), values)?;
        let collection = self.collection_handle();
        let held = self.held.as_ref().map(|held| match held.chosen {
            true => held.clone(),
            false => crate::versions::Held {
                commit: held.commit.clone(),
                failed: collection.blob("HEAD", &self.relative_path()).unwrap_or_default(),
                ..Default::default()
            },
        });
        let commit = match &self.held {
            Some(held) => held.commit.clone(),
            None => collection.head()?,
        };
        crate::versions::installed(&state, &self.key(), &commit, held)
    }

    /// Installer files this install uses that aren't checked out yet (see blobs_on_demand).
    fn check_out_installers(&self, values: &serde_json::Map<String, Value>) -> Result<()> {
        let files = self.missing_installers(values);
        if files.is_empty() {
            return Ok(());
        }
        self.collection_handle().check_out(&files)
    }

    fn collection_handle(&self) -> Collection {
        Collection { name: self.collection.clone(), dir: self.collection_dir.clone(), managed: self.managed }
    }

    /// The app's file, relative to the collection, e.g. `apps/gimp.dsc.yaml`.
    fn relative_path(&self) -> String {
        self.path.strip_prefix(&self.collection_dir).unwrap_or(&self.path).to_string_lossy().into_owned()
    }

    /// Source trees in an historical app are materialized from the same revision as its YAML.
    fn directory_sources(&self, values: &serde_json::Map<String, Value>) -> Result<Vec<String>> {
        fn collect(list: &Value, root: &Path, doc: &Value, values: &serde_json::Map<String, Value>, out: &mut Vec<String>) -> Result<()> {
            for entry in list.as_array().into_iter().flatten() {
                if entry["condition"].as_str().is_some_and(|c| !crate::conditions::holds_for(doc, c, values).unwrap_or(true)) { continue; }
                if entry["type"] == PACKAGE_TYPE && let Some(folder) = entry["properties"]["flake"].as_str() {
                    out.push(folder.to_owned());
                }
                if entry["type"] == DIRECTORY_TYPE && let Some(source) = entry["properties"]["source"].as_str() {
                    out.push(Path::new(source).strip_prefix(root)?.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/"));
                }
                collect(&entry["properties"]["resources"], root, doc, values, out)?;
            }
            Ok(())
        }
        let mut out = Vec::new();
        let root = plain_path(self.collection_dir.canonicalize()?);
        collect(&self.doc["resources"], &root, &self.doc, values, &mut out)?;
        collect(&self.doc["metadata"]["furnish"]["uninstall"], &root, &self.doc, values, &mut out)?;
        Ok(out)
    }

    /// Installer files (relative to the collection) the resources and removal steps that apply
    /// with these values use and that aren't on disk.
    fn missing_installers(&self, values: &serde_json::Map<String, Value>) -> Vec<String> {
        let mut files = Vec::new();
        let steps = crate::engine::removal_steps(&self.doc).into_iter().flatten();
        for resource in self.doc["resources"].as_array().into_iter().flatten().chain(steps) {
            let file = match resource["type"].as_str() {
                Some(INSTALLER_TYPE) => resource["properties"]["file"].as_str(),
                Some(FILE_TYPE | COMMAND_TYPE) => resource["properties"]["source"].as_str(),
                _ => None,
            };
            let Some(file) = file else { continue };
            let applies = resource["condition"].as_str().is_none_or(|c| crate::conditions::holds_for(&self.doc, c, values).unwrap_or(true));
            // Paths of files not checked out start from the canonical directory (file_in_repo_later).
            let base = self.collection_dir.canonicalize().map(plain_path).unwrap_or_else(|_| self.collection_dir.clone());
            if applies && !Path::new(file).is_file()
                && let Ok(relative) = Path::new(file).strip_prefix(&base).or_else(|_| Path::new(file).strip_prefix(&self.collection_dir))
            {
                files.push(relative.to_string_lossy().into_owned());
            }
        }
        files
    }

    /// The version it is held at, if it is (see `held`).
    pub fn held_version(&self) -> Option<Version> {
        let held = self.held.as_ref()?;
        match self.collection_handle().commit_info(&held.commit) {
            Ok((commit, _, date)) => Some(Version { commit, date, ..Default::default() }),
            // No longer in the history kept (checkoutDepth): the day it was installed with.
            Err(_) => Some(Version { commit: held.commit.clone(), date: held.date.clone().unwrap_or_default(), ..Default::default() }),
        }
    }

    /// Held at an earlier version, and the collection's current file for the app differs from it.
    pub fn newer_than_held(&self) -> bool {
        let Some(held) = &self.held else { return false };
        let collection = self.collection_handle();
        let relative = self.relative_path();
        collection.blob("HEAD", &relative).is_none_or(|now| collection.blob(&held.commit, &relative).is_none_or(|then| then != now))
    }

    /// Whether the version it is held at offers no acceptable version (see `required`) where the
    /// collection's current version requires some.
    fn held_offers_nothing_acceptable(&self, values: &serde_json::Map<String, Value>) -> Result<bool> {
        if !self.current()?.resources_here().iter().any(|r| r["properties"].get(crate::required::PROPERTY).is_some()) {
            return Ok(false);
        }
        Ok(crate::engine::check(&self.doc_for(values))?.required.is_some_and(|r| r.target.is_none()))
    }

    /// Choosing versions must be on for the collection (setting `chooseVersion`).
    fn require_choosing(&self) -> Result<()> {
        if !crate::settings::for_collection(&self.collection).settings.choose_version {
            bail!("choosing a version of an app is turned off for collection '{}' (chooseVersion)", self.collection);
        }
        Ok(())
    }

    /// The commits within the history kept (checkoutDepth) that changed the app's file, newest
    /// first, with the time of each and the file's blob id there. The first is the current version.
    fn history(&self) -> Result<Vec<(Version, i64, String)>> {
        let history = self.collection_handle().file_history(&self.relative_path());
        if history.is_empty() {
            bail!("{} isn't in the history of collection '{}'", self.id, self.collection);
        }
        Ok(history
            .into_iter()
            .enumerate()
            .map(|(i, (commit, time, date, blob))| (Version { commit, date, stated: Vec::new(), current: i == 0 }, time, blob))
            .collect())
    }

    /// Why a version older than `fallbackMaxAgeDays` allows isn't offered; `None` if it may be.
    fn too_old(&self, version: &Version, time: i64, newest: i64) -> Option<String> {
        let days = crate::settings::for_collection(&self.collection).settings.fallback_max_age_days;
        older_than(days, time, newest).then(|| format!("the version of {} from {} is more than {days} days older than the current one (fallbackMaxAgeDays)", self.id, version.date))
    }

    /// The versions people can choose from (see the setting `chooseVersion`), newest first, the
    /// current one first: the commits that changed the app's file within the history kept
    /// (checkoutDepth), at most fallbackMaxAgeDays older than the current one, signed by a key
    /// the organization trusts (trustedKeys), and with an acceptable version where the
    /// collection requires versions. Asks the furnish service for apps it installs.
    pub fn versions(&self) -> Result<Vec<Version>> {
        self.require_choosing()?;
        let values = self.installed_values();
        if self.remote(&values) {
            return crate::service::versions(&self.key());
        }
        let history = self.history()?;
        let newest = history[0].1;
        let history: Vec<_> = history.into_iter().filter(|(v, time, _)| v.current || self.too_old(v, *time, newest).is_none()).collect();
        let collection = self.collection_handle();
        let contents = collection.blobs(&history.iter().map(|(_, _, blob)| blob.as_str()).collect::<Vec<_>>());
        let current = self.current()?;
        let mut judged = BTreeMap::new();
        let mut versions = Vec::new();
        for (mut version, _, blob) in history {
            let file = contents.get(&blob).and_then(|b| serde_yaml::from_slice::<Value>(b).ok());
            version.stated = file.as_ref().map(stated_versions).unwrap_or_default();
            if version.current || self.acceptable(&version, file, &current, &values, &mut judged).is_ok() {
                versions.push(version);
            }
        }
        Ok(versions)
    }

    /// The version given as a day (2026-11-02) or a commit (at least 4 characters of
    /// its id), among the commits that changed the app's file (see `history`).
    pub fn resolve_version(&self, query: &str) -> Result<Version> {
        let history = self.history()?;
        let by_day = query.len() == 10 && query.as_bytes()[4] == b'-' && query.as_bytes()[7] == b'-';
        let found: Vec<&Version> = history
            .iter()
            .map(|(v, _, _)| v)
            .filter(|v| if by_day { v.date == query } else { query.len() >= 4 && v.commit.starts_with(&query.to_ascii_lowercase()) })
            .collect();
        match found.as_slice() {
            [one] => Ok((*one).clone()),
            [] => bail!("{} has no version {} {query} (see furnish versions {})", self.id, if by_day { "from" } else { "with the commit" }, self.id),
            more => bail!(
                "{} has {} versions from {query}; give the commit instead: {}",
                self.id,
                more.len(),
                more.iter().map(|v| short(&v.commit)).collect::<Vec<_>>().join(", ")
            ),
        }
    }

    /// Whether an earlier version may be installed: signed by a trusted key, and offering an
    /// acceptable version where the collection's current version requires versions (see
    /// `previous_version`). `file`: the app's file at that commit, if read. `judged`: earlier
    /// results by what decides them (the resources with versions and their installer files), so
    /// versions that didn't change those aren't checked again.
    fn acceptable(
        &self,
        version: &Version,
        file: Option<Value>,
        current: &App,
        values: &serde_json::Map<String, Value>,
        judged: &mut BTreeMap<String, Result<(), String>>,
    ) -> Result<()> {
        let collection = self.collection_handle();
        collection.verify(&version.commit)?;
        if !current.resources_here().iter().any(|r| r["properties"].get(crate::required::PROPERTY).is_some()) {
            return Ok(());
        }
        let key = if let Some(mut file) = file {
            crate::required::carry(&current.doc, &mut file);
            let decisive: Vec<Value> = file["resources"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|r| r["properties"].get(crate::required::PROPERTY).is_some())
                .map(|r| {
                    let mut r = r.clone();
                    if let Some(installer) = r["properties"]["file"].as_str().filter(|_| r["type"].as_str().is_some_and(|t| t.eq_ignore_ascii_case(INSTALLER_TYPE))) {
                        r["properties"]["file"] = collection.blob(&version.commit, installer).unwrap_or_default().into();
                    }
                    r
                })
                .collect();
            Some(serde_json::to_string(&(decisive, collection.manager_blobs(&version.commit)?))?)
        } else { None };
        if let Some(earlier) = key.as_ref().and_then(|k| judged.get(k)) {
            return earlier.clone().map_err(|e| anyhow::anyhow!(e));
        }
        let versions = data_dir().join("versions").join(&self.collection);
        fs::create_dir_all(&versions)?;
        let scratch = tempfile::Builder::new().prefix(".preview-").tempdir_in(&versions)?;
        let result = (|| {
            let earlier = self.earlier_at(version, values, scratch.path())?;
            if let Some(required) = crate::engine::check(&earlier.doc_for(values))?.required.filter(|r| r.target.is_none()) {
                bail!("the version of {} from {} has no acceptable version: needs {}", self.id, version.date, required.summary);
            }
            Ok(())
        })();

        if let Some(key) = key {
            judged.insert(key, result.as_ref().map(|_| ()).map_err(|e| format!("{e:#}")));
        }
        result
    }

    /// Install a version the person chose (a commit, see `versions`) and hold the app there.
    /// The current version installs as usual; an earlier one only with `chooseVersion` on, and
    /// within the limits `versions` lists by.
    fn install_chosen_with(&self, commit: &str, values: &serde_json::Map<String, Value>, report: impl FnMut(crate::engine::Progress)) -> Result<()> {
        let history = self.history()?;
        let newest = history[0].1;
        let Some((version, time, _)) = history.into_iter().find(|(v, _, _)| v.commit == commit) else {
            bail!("{} has no version with the commit {} in the history furnish keeps of collection '{}'", self.id, short(commit), self.collection);
        };
        if version.current {
            return self.current()?.install_with(values, report);
        }
        self.require_choosing()?;
        if let Some(why) = self.too_old(&version, time, newest) {
            bail!(why);
        }
        self.acceptable(&version, None, &self.current()?, values, &mut BTreeMap::new())?;
        let earlier = self.earlier(&version, values)?;
        let doc = earlier.doc_here(values);
        crate::engine::run(&doc, false, report)?;
        earlier.require_installed(&doc)?;
        let state = self.state_dir(values);
        crate::options::remember(&state, &self.key(), values)?;
        let held = crate::versions::Held { commit: version.commit.clone(), chosen: true, date: Some(version.date.clone()), ..Default::default() };
        crate::versions::installed(&state, &self.key(), &version.commit, Some(held))
    }

    /// The app as the collection has it now, also if it is held at an earlier version.
    pub fn current(&self) -> Result<App> {
        self.collection_handle().load_app(&self.path)
    }

    /// The record of the versions it was installed from: the user's, else the service's.
    fn versions_record(&self) -> crate::versions::Record {
        let own = crate::versions::get(&data_dir(), &self.key());
        if own.good.is_none() && self.managed && crate::service::enabled() {
            return crate::versions::get(&system_dir(), &self.key());
        }
        own
    }

    /// The earlier version to fall back to: the one that last installed without error here,
    /// else the app's file before its latest change. Within the limits of the fallback settings.
    pub fn previous_version(&self) -> Result<Version> {
        let settings = crate::settings::for_collection(&self.collection).settings;
        if settings.fallback == crate::settings::Fallback::Off {
            bail!("falling back to an earlier version is turned off for collection '{}'", self.collection);
        }
        let collection = self.collection_handle();
        let relative = self.relative_path();
        let current = collection.blob("HEAD", &relative).with_context(|| format!("{} isn't in the collection", self.id))?;
        let changes = collection.changes(&relative, 2);
        let good = self.versions_record().good.filter(|g| collection.blob(g, &relative).is_some_and(|b| b != current));
        let (commit, time, date) = match good {
            Some(good) => collection.commit_info(&good)?,
            None => match changes.get(1) {
                Some(change) => change.clone(),
                None if settings.checkout_depth > 0 => bail!(
                    "{} has no earlier version in the {} commits furnish keeps of collection '{}' (checkoutDepth)",
                    self.id,
                    settings.checkout_depth,
                    self.collection
                ),
                None => bail!("{} has no earlier version", self.id),
            },
        };
        let days = settings.fallback_max_age_days;
        if changes.first().is_some_and(|(_, newest, _)| older_than(days, time, *newest)) {
            bail!("the previous version of {} is from {date}, more than {days} days older than the current one", self.id);
        }
        collection.verify(&commit)?;
        let version = Version { commit, date, ..Default::default() };
        // Never back to a version the collection's required versions no longer accept.
        let current = self.current()?;
        let values = self.installed_values();
        if current.resources_here().iter().any(|r| r["properties"].get(crate::required::PROPERTY).is_some()) {
            let earlier = self.earlier(&version, &values)?;
            if let Some(required) = crate::engine::check(&earlier.doc_for(&values))?.required.filter(|r| r.target.is_none()) {
                bail!("the previous version of {} from {} has no acceptable version: needs {}", self.id, version.date, required.summary);
            }
        }
        Ok(version)
    }

    /// The app at an earlier version, with the versions the collection requires now (see
    /// `required::carry`). Its files come from that commit, kept until no app is held there.
    fn earlier(&self, version: &Version, values: &serde_json::Map<String, Value>) -> Result<App> {
        self.earlier_at(version, values, &version_dir(&self.state_dir(values), &self.collection, &version.commit))
    }

    /// `earlier`, with the files of that commit in `dir`.
    fn earlier_at(&self, version: &Version, values: &serde_json::Map<String, Value>, dir: &Path) -> Result<App> {
        let collection = self.collection_handle();
        let relative = self.relative_path();
        let dir = dir.to_path_buf();
        collection.extract(&version.commit, &relative, values, &dir).with_context(|| format!("getting the version from {}", version.date))?;
        let file = dir.join(&relative);
        let mut earlier = Collection { name: self.collection.clone(), dir, managed: self.managed }
            .load_scoped_app(&file, allowed_resource_types().as_deref(), &self.package_scope)?;
        crate::required::carry(&self.current()?.doc, &mut earlier.doc);
        Ok(earlier)
    }

    /// Install the previous version (see `previous_version`) and hold the app there.
    /// Only after installing the collection's current version of the app failed here (or while
    /// it is held at an earlier one because of that).
    fn install_previous_with(&self, values: &serde_json::Map<String, Value>, report: impl FnMut(crate::engine::Progress)) -> Result<()> {
        let version = self.previous_version()?;
        let collection = self.collection_handle();
        let relative = self.relative_path();
        let state = self.state_dir(values);
        let current = collection.blob("HEAD", &relative).unwrap_or_default();
        if !crate::versions::get(&state, &self.key()).has_failed(&current) {
            bail!("{} goes back to an earlier version only after installing its current version failed", self.id);
        }
        let earlier = self.earlier(&version, values)?;
        let doc = earlier.doc_here(values);
        crate::engine::run(&doc, false, report)?;
        earlier.require_installed(&doc)?;
        crate::options::remember(&state, &self.key(), values)?;
        let failed = collection.blob("HEAD", &relative).unwrap_or_default();
        let held = crate::versions::Held { commit: version.commit.clone(), failed, ..Default::default() };
        crate::versions::installed(&state, &self.key(), &version.commit, Some(held))
    }

    /// Uninstall with the values it was installed with.
    pub fn uninstall(&self, report: impl FnMut(crate::engine::Progress)) -> Result<()> {
        crate::reports::operation("uninstall", self, None, || self.uninstall_unreported(report))
    }

    fn uninstall_unreported(&self, report: impl FnMut(crate::engine::Progress)) -> Result<()> {
        let values = self.installed_values();
        if self.remote(&values) {
            return crate::service::run(&crate::service::Request::Uninstall { app: self.key() }, report);
        }
        let _lock = self.collection_handle().lock()?;
        let state = self.state_dir(&values);
        self.remove_with(&values, report)?;
        crate::package_receipts::forget(&state, &self.key(), &self.package_scope)?;
        crate::versions::forget(&state, &self.key())?;
        crate::options::forget(&state, &self.key())
    }

    /// Apply changed options: uninstall with the old values, install with the current ones.
    /// Moving between a system-wide and a per-user install takes both the service and this process.
    pub fn reinstall(&self, report: impl FnMut(crate::engine::Progress)) -> Result<()> {
        let version = self.collection_handle().head().ok();
        crate::reports::operation("reinstall", self, version, || self.reinstall_unreported(report))
    }

    fn reinstall_unreported(&self, mut report: impl FnMut(crate::engine::Progress)) -> Result<()> {
        self.require_answers()?;
        let (old, new) = (self.installed_values(), self.current_values());
        // Removal follows the installed owner. After that receipt is cleared, the
        // new install selects its own manager and may need a different account.
        let new_remote = self.managed && crate::service::enabled() && !self.per_user_in(&new, &[]);
        let old_remote = self.remote(&old);
        if old_remote && new_remote {
            let install_into = crate::nixos_apps::own_install_into();
            return crate::service::run(&crate::service::Request::Reinstall { app: self.key(), values: new, install_into }, report);
        }
        if old_remote || new_remote {
            crate::engine::preflight_install(&self.doc_for(&new))?;
            self.uninstall_unreported(&mut report)?;
            return self.install_values(new, Which::Shown, report);
        }
        self.reinstall_with(&new, report)
    }

    pub fn reinstall_with(&self, values: &serde_json::Map<String, Value>, mut report: impl FnMut(crate::engine::Progress)) -> Result<()> {
        let collection = self.collection_handle();
        let _lock = collection.lock()?;
        collection.verify(&collection.head()?)?;
        let old = self.installed_values();
        crate::engine::preflight_install(&self.doc_here(values))?;
        let state = self.state_dir(&old);
        self.remove_with(&old, &mut report)?;
        crate::package_receipts::forget(&state, &self.key(), &self.package_scope)?;
        crate::options::forget(&state, &self.key())?;
        self.install_with(values, report)
    }
}

/// An error that is also in the operation's log (the Activity log in the desktop app).
fn logged(message: String) -> anyhow::Error {
    crate::engine::emit_log("error", message.clone());
    anyhow::anyhow!(message)
}

pub fn all_kits() -> Result<Vec<Kit>> {
    Ok(collections()?.iter().flat_map(Collection::kits).collect())
}

/// Find a kit by `id` or `collection/id`.
pub fn find_kit(query: &str) -> Result<Kit> {
    let (collection, id) = match query.split_once('/') {
        Some((c, id)) => (Some(c), id),
        None => (None, query),
    };
    let mut matches: Vec<_> = all_kits()?
        .into_iter()
        .filter(|k| k.id == id && collection.is_none_or(|c| k.collection == c))
        .collect();
    match matches.len() {
        0 => bail!("no app or kit named '{query}' (see `furnish list`)"),
        1 => Ok(matches.remove(0)),
        _ => bail!("'{query}' is ambiguous, use collection/{id}"),
    }
}

/// Apps to preselect when uninstalling `kit`: its installed items, except those another
/// installed kit (all its required items installed) also contains.
pub fn default_uninstall<'a>(kit: &'a Kit, kits: &[Kit], installed: &dyn Fn(&str, &str) -> bool) -> Vec<&'a str> {
    let others: Vec<&Kit> = kits
        .iter()
        .filter(|k| !(k.collection == kit.collection && k.id == kit.id))
        .filter(|k| k.items.iter().filter(|i| i.required).all(|i| installed(&k.collection, &i.app)))
        .filter(|k| k.items.iter().any(|i| installed(&k.collection, &i.app)))
        .collect();
    kit.items
        .iter()
        .map(|i| i.app.as_str())
        .filter(|app| installed(&kit.collection, app))
        .filter(|app| !others.iter().any(|k| k.collection == kit.collection && k.items.iter().any(|i| i.app == *app)))
        .collect()
}

pub fn all_apps() -> Result<Vec<App>> {
    Ok(collections()?.iter().flat_map(Collection::apps).collect())
}

/// Find an app by `id` or `collection/id`.
pub fn find_app(query: &str) -> Result<App> {
    let (collection, id) = match query.split_once('/') {
        Some((c, id)) => (Some(c), id),
        None => (None, query),
    };
    let mut matches: Vec<_> = all_apps()?
        .into_iter()
        .filter(|a| a.id == id && collection.is_none_or(|c| a.collection == c))
        .collect();
    match matches.len() {
        0 => bail!("no app named '{query}' (see `furnish list`)"),
        1 => Ok(matches.remove(0)),
        _ => {
            let options: Vec<_> = matches.iter().map(|a| format!("{}/{}", a.collection, a.id)).collect();
            bail!("'{query}' is ambiguous, use one of: {}", options.join(", "))
        }
    }
}

/// Regular files relative to a Git directory at a particular revision, including blobs left out of a sparse checkout.
pub(crate) fn directory_tree(repo: &Path, revision: &str, folder: &str) -> Result<Vec<(String, bool)>> {
    let spec = format!("{revision}:{folder}");
    let bytes = git_bytes(Some(repo), &["ls-tree", "-r", "-z", "--end-of-options", &spec])?;
    std::str::from_utf8(&bytes)?.split_terminator('\0').map(|entry| {
        let (info, name) = entry.split_once('\t').context("invalid Git tree entry")?;
        anyhow::ensure!(info.starts_with("100644 blob ") || info.starts_with("100755 blob "), "{folder}/{name} must be a regular file");
        Ok((name.to_string(), info.starts_with("100755 ")))
    }).collect()
}

/// Installer files, which blobs_on_demand leaves out of the checkout until an app needs one.
const INSTALLERS: [&str; 6] = ["*.msi", "*.exe", "*.pkg", "*.dmg", "*.deb", "*.rpm"];

/// Variables that point git at another repository than the one `-C` names, as `git rebase -x`
/// and git hooks set them for the commands they run. furnish's git commands (and its tests')
/// never inherit them, so they can't write into whatever repository furnish was started from.
pub const GIT_LOCATION_VARS: [&str; 6] =
    ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR", "GIT_OBJECT_DIRECTORY", "GIT_ALTERNATE_OBJECT_DIRECTORIES"];

/// The git program (see [`git_program`]), without the variables in [`GIT_LOCATION_VARS`].
pub fn git_command() -> Command {
    let mut cmd = git_program();
    // Git and its smudge filters must find the same helper, including when Finder or launchd
    // started the app with only system tools on PATH.
    let helper_path = option_env!("FURNISH_GIT_PATH").map(std::ffi::OsString::from);
    #[cfg(target_os = "macos")]
    let helper_path = helper_path.or_else(|| {
        let exe = env::current_exe().ok()?.canonicalize().ok()?;
        let bin = exe.parent()?;
        let helper = bin.join("git-lfs");
        (bin.ends_with("Contents/MacOS") && helper.is_file()
            && (!crate::service::in_service() || crate::service::admin_only(&helper)))
            .then(|| bin.as_os_str().to_owned())
    });
    if let Some(prefix) = helper_path {
        let inherited = env::var_os("PATH").unwrap_or_default();
        if let Ok(path) = env::join_paths(env::split_paths(&prefix).chain(env::split_paths(&inherited))) {
            cmd.env("PATH", path);
        }
    }
    cmd
}

fn git_process(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut cmd = Command::new(program);
    for var in GIT_LOCATION_VARS {
        cmd.env_remove(var);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW for GUI callers.
    }
    cmd
}

#[cfg(not(windows))]
fn git_program() -> Command {
    // Root uses the system Git. On NixOS it comes from the service's PATH.
    if crate::service::in_service() && Path::new("/usr/bin/git").is_file() {
        return git_process("/usr/bin/git");
    }
    git_process("git")
}

#[cfg(windows)]
fn git_program() -> Command {
    static GIT: std::sync::OnceLock<(PathBuf, bool)> = std::sync::OnceLock::new();
    let (program, bundled) = GIT.get_or_init(|| {
        let path = env::var_os("PATH").unwrap_or_default();
        let system = env::split_paths(&path).filter(|dir| dir.is_absolute()).map(|dir| dir.join("git.exe")).find(|p| p.is_file());
        let packaged = env::current_exe().ok().and_then(|exe| Some(exe.parent()?.join("git/cmd/git.exe")));
        if let Some(git) = system
            && (packaged.as_ref().is_none_or(|p| !p.is_file()) || git_process(&git).args(["lfs", "version"]).output().is_ok_and(|r| r.status.success()))
        {
            return (git, false);
        }
        (packaged.unwrap_or_else(|| PathBuf::from("git")), true)
    });
    let mut cmd = git_process(program);
    if !*bundled { return cmd; }
    // furnish asks for logins itself; the bundled credential manager mustn't open windows.
    cmd.env("GCM_INTERACTIVE", "never");
    // Windows' own OpenSSH, so its ssh-agent service and keys work like with a system git.
    let openssh = env::var_os("SystemRoot").map(|r| PathBuf::from(r).join("System32").join("OpenSSH").join("ssh.exe"));
    if let Some(ssh) = openssh.filter(|s| s.is_file())
        && env::var_os("GIT_SSH_COMMAND").is_none()
    {
        cmd.env("GIT_SSH_COMMAND", format!("\"{}\"", ssh.display()).replace('\\', "/"));
    }
    cmd
}

/// Why git couldn't run, and what to do about it.
fn git_missing(err: std::io::Error) -> anyhow::Error {
    if err.kind() != std::io::ErrorKind::NotFound {
        return anyhow::Error::new(err).context("failed to run git");
    }
    let how = if cfg!(windows) {
        "reinstall furnish (its installer includes git), or install Git for Windows"
    } else if cfg!(target_os = "macos") {
        "install Apple's Command Line Tools (run `xcode-select --install`) or git from Homebrew"
    } else {
        "install the package git with your package manager"
    };
    anyhow::anyhow!("furnish needs git, which isn't installed: {how}")
}

fn git(dir: Option<&Path>, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8_lossy(&git_bytes(dir, args)?).trim().to_string())
}

pub(crate) fn git_bytes(dir: Option<&Path>, args: &[&str]) -> Result<Vec<u8>> {
    git_bytes_input(dir, args, None)
}

fn git_bytes_input(dir: Option<&Path>, args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>> {
    let _working = crate::load::activity(); // load-aware updates ignore furnish's own downloads
    let mut cmd = git_command();
    if let Some(dir) = dir {
        cmd.arg("-C").arg(dir);
        // The service's collections belong to root/SYSTEM; git refuses other users' repositories otherwise.
        if dir.starts_with(system_dir()) {
            cmd.args(["-c", "safe.directory=*"]);
        }
    }
    // Talking to the server: never a terminal prompt (an app has none), and furnish's credentials
    // for that URL, if any (see `auth`); SSH keys, the agent and credential helpers work as usual.
    // Commands that write or read files download the contents they lack (see blobs_on_demand).
    // The command: the first argument after `-c <name>=<value>` pairs.
    let command = args.iter().enumerate().find(|(i, a)| **a != "-c" && (*i == 0 || args[i - 1] != "-c")).map(|(_, a)| *a);
    let network = matches!(command, Some("clone" | "fetch" | "pull" | "lfs" | "checkout" | "reset" | "read-tree" | "cat-file"))
        || (command == Some("ls-remote") && args.get(1) != Some(&"--get-url"));
    let url = if !network {
        None
    } else if command == Some("clone") || (command == Some("ls-remote") && dir.is_none()) {
        match args.iter().position(|a| *a == "--end-of-options") {
            Some(end) => args.get(end + 1).map(|a| a.to_string()),
            None => args.iter().skip(1).find(|a| !a.starts_with('-')).map(|a| a.to_string()),
        }
    } else {
        dir.and_then(|d| origin_url(d).ok())
    };
    if let Some(url) = &url {
        cmd.envs(crate::auth::git_env(url)?);
    }
    if let Some(input) = input {
        // The tag-date fetch reads Git's dumb HTTP capability diagnostic to select its fallback.
        cmd.env("LC_ALL", "C");
        let mut file = tempfile::tempfile()?;
        file.write_all(input)?;
        file.rewind()?;
        cmd.stdin(file);
    }
    let output = cmd.args(args).output().map_err(git_missing)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        // macOS: /usr/bin/git is a placeholder until the Command Line Tools are installed.
        if stderr.contains("xcrun: error: invalid active developer path") || stderr.contains("No developer tools were found") {
            return Err(git_missing(std::io::ErrorKind::NotFound.into()));
        }
        if let Some(url) = url.filter(|u| u.starts_with("http") && crate::auth::is_auth_failure(&stderr)) {
            return Err(crate::auth::CredentialsRequired { url }.into());
        }
        if let Some(advice) = crate::auth::ssh_advice(&stderr) {
            bail!("git {} failed: {advice} ({stderr})", args.join(" "));
        }
        bail!("git {} failed: {stderr}", args.join(" "));
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn why_the_service_doesnt_install_an_app_is_decided_here() {
        use crate::nixos_apps::OutsideService;
        let why = |in_service, service, managed, per_user| outside_service(in_service, service, managed, "mine", "htop", || per_user);
        assert_eq!(why(true, false, false, true), None, "the service itself");
        assert_eq!(why(false, false, true, false), Some(OutsideService::Off), "no service section in the policy");
        assert_eq!(why(false, true, false, false), Some(OutsideService::NotPreset { collection: "mine".into() }));
        assert_eq!(why(false, true, true, true), Some(OutsideService::PerUser { app: "htop".into() }));
        assert_eq!(why(false, true, true, false), None, "through the service");
    }

    #[test]
    fn only_collections_an_endpoint_lists_anew_are_added_by_the_rules_for_adding() {
        use crate::endpoint::Collection as Listed;
        let e = |name: &str| Listed { name: name.into(), url: format!("https://furnish.acme.tld/git/{name}.git"), tracking: None };
        let listed = [e("tools"), e("removed"), e("new"), e("mine"), e("same"), e("it"), e("preset"), e("CON")];
        // Added before (and then removed by the person, or not): never again.
        let offered = ["tools".to_string(), "removed".to_string()];
        let presets = [
            crate::policy::Preset { name: "it".into(), url: "https://git.acme.tld/it.git".into(), tracking: None },
            crate::policy::Preset { name: "preset".into(), url: e("preset").url, tracking: None },
        ];
        let local = |name: &str| match name {
            "tools" => Some(e("tools").url),
            "mine" => Some("https://github.com/me/mine.git".into()),
            "same" => Some(e("same").url),
            _ => None,
        };
        assert_eq!(
            offers(&listed, &offered, &presets, local),
            [
                Offer::Add(&listed[2]),
                Offer::Refused("a collection named 'mine' already exists".into()),
                Offer::Present("same"),
                Offer::Refused("'it' is the name of a collection your organization provides".into()),
                // "preset": the admin's, from the same URL, set up by ensure_presets.
                Offer::Refused("invalid collection name 'CON': Windows reserves it for a device".into()),
            ]
        );
        assert!(offers(&listed, &listed.iter().map(|c| c.name.clone()).collect::<Vec<_>>(), &presets, local).is_empty());
    }

    #[test]
    fn every_configured_origin_must_qualify_for_a_preset_or_a_personal_collection() {
        let repo = tempfile::tempdir().unwrap();
        let dir = Some(repo.path());
        git(dir, &["init", "--quiet"]).unwrap();
        let preset = crate::policy::Preset { name: "it".into(), url: "approved".into(), tracking: None };
        let policy = serde_json::from_str(r#"{"allowedUrlPatterns":["approved"]}"#).unwrap();
        let collection = Collection { name: "it".into(), dir: repo.path().into(), managed: false };
        let presets = [preset];
        assert!(!personal_origin_allowed(&policy, &collection));
        git(dir, &["remote", "add", "origin", "approved"]).unwrap();
        assert!(is_preset(&presets, "it", repo.path()) && personal_origin_allowed(&policy, &collection));
        git(dir, &["config", "--add", "remote.origin.url", "elsewhere"]).unwrap();
        assert!(!personal_origin_allowed(&policy, &collection));
        git(dir, &["config", "--add", "remote.origin.url", "approved"]).unwrap();
        assert!(!is_preset(&presets, "it", repo.path()));
    }

    #[test]
    fn collection_name_from_url() {
        assert_eq!(name_from_url("https://gitlab.com/furnish-self-service/self-service-example.git"), "self-service-example");
        assert_eq!(name_from_url("git@gitlab.com:org/apps.git"), "apps");
        assert_eq!(name_from_url("/srv/git/apps/"), "apps");
    }

    fn kit(collection: &str, id: &str, items: &[(&str, bool)]) -> Kit {
        Kit {
            id: id.into(),
            collection: collection.into(),
            name: None,
            description: String::new(),
            categories: vec![],
            icon: None,
            icon_dark: None,
            recommendation: Value::Null,
            collection_dir: PathBuf::new(),
            items: items.iter().map(|(app, required)| Item { app: (*app).into(), required: *required, selected: true, condition: None }).collect(),
        }
    }

    #[test]
    fn uninstall_keeps_items_another_installed_kit_needs() {
        let web = kit("c", "web", &[("editor", true), ("node", false), ("docker", false)]);
        let ops = kit("c", "ops", &[("docker", true), ("kubectl", false)]);
        let design = kit("c", "design", &[("figma", true), ("node", false)]); // not installed: figma missing
        let installed = |_: &str, app: &str| ["editor", "node", "docker", "kubectl"].contains(&app);
        let kits = [web, ops, design];
        // docker stays (ops is installed); node goes (design isn't installed).
        assert_eq!(default_uninstall(&kits[0], &kits, &installed), ["editor", "node"]);
    }

    #[test]
    fn kit_file_defaults() {
        let file: KitFile = serde_yaml::from_str("items:\n  - app: a\n    required: true\n  - app: b\n  - app: c\n    selected: false\n").unwrap();
        let k = Kit { recommendation: file.recommended, items: file.items, ..kit("c", "k", &[]) };
        assert!(!k.recommended());
        assert_eq!(default_selection(&k.items), ["a", "b"]);
        let with = ["c".to_string()];
        let without = ["b".to_string()];
        assert_eq!(choose(&k.items, &with, &without, "kit k").unwrap(), ["a", "c"]);
        assert!(choose(&k.items, &[], &["a".to_string()], "kit k").is_err()); // required
        assert!(choose(&k.items, &["x".to_string()], &[], "kit k").is_err()); // unknown
    }

    #[test]
    fn rejects_collection_names_that_escape_the_data_dir() {
        for name in ["../evil", "a/b", "..", ".hidden", ""] {
            assert!(add("https://example.com/x.git", name, None).is_err(), "{name} accepted");
        }
    }

    #[test]
    fn rejects_collection_names_windows_reserves() {
        for name in ["CON", "nul", "Aux.apps", "com1", "LPT9", "apps.", "a b"] {
            assert!(check_name(name).is_err(), "{name} accepted");
        }
        for name in ["console", "com10", "apps.v2", "nul-apps", "it_team"] {
            assert!(check_name(name).is_ok(), "{name} refused");
        }
    }

    #[test]
    fn sparse_patterns_match_only_the_file() {
        assert_eq!(sparse_pattern("installers/a.deb").unwrap(), "/installers/a.deb\n");
        assert_eq!(sparse_pattern("i/*.deb").unwrap(), "/i/\\*.deb\n");
        assert_eq!(sparse_pattern("i/[x]?!#\\ .msi").unwrap(), "/i/\\[x\\]\\?\\!\\#\\\\\\ .msi\n");
        assert!(sparse_pattern("a\n/*").is_err());
    }

    #[test]
    fn git_commands_ignore_an_inherited_repository() {
        let cmd = git_command();
        let removed: Vec<_> = cmd.get_envs().filter(|(_, v)| v.is_none()).map(|(k, _)| k.to_string_lossy().into_owned()).collect();
        for var in GIT_LOCATION_VARS {
            assert!(removed.iter().any(|r| r == var), "{var} is passed on");
        }
    }

    #[test]
    fn an_update_is_available_until_its_commit_is_checked_out() {
        let dir = env::temp_dir().join(format!("furnish-remote-{}", unique()));
        let (origin, copy, file) = (dir.join("origin"), dir.join("copy"), dir.join("remote-heads.json"));
        fs::create_dir_all(&origin).unwrap();
        let commit = |message: &str| {
            git(Some(&origin), &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", message]).unwrap();
        };
        git(Some(&origin), &["init", "-q"]).unwrap();
        commit("one");
        git(None, &["clone", "-q", "--end-of-options", &origin.to_string_lossy(), &copy.to_string_lossy()]).unwrap();
        let head = || git(Some(&copy), &["rev-parse", "HEAD"]).unwrap();
        let server = || Target::resolve(Some(&copy), "origin", &Tracking::Default);
        let look = || remember_remote_head(&file, "c", "source", &server().unwrap().commit);
        fs::write(&file, r#"{"c":{"commit":"older","checkedAt":100}}"#).unwrap();
        assert!(!remote_head_differs(&file, "c", "source", &head()), "a legacy notice has no verified source");

        assert!(!remote_head_differs(&file, "c", "source", &head()), "never looked");
        look();
        assert!(!remote_head_differs(&file, "c", "source", &head()), "up to date");
        commit("two");
        look();
        assert!(remote_head_differs(&file, "c", "source", &head()), "the server moved on");
        assert!(!remote_head_differs(&file, "c", "another selector", &head()), "notice belonged to another source");
        assert!(!remote_head_differs(&file, "other", "source", &head()), "per collection");
        // Offline: no answer, and what was seen before stays.
        fs::rename(&origin, dir.join("gone")).unwrap();
        assert!(server().is_err());
        assert!(remote_head_differs(&file, "c", "source", &head()));
        fs::rename(dir.join("gone"), &origin).unwrap();
        // Applied: the indicator goes.
        git(Some(&copy), &["pull", "-q"]).unwrap();
        assert!(!remote_head_differs(&file, "c", "source", &head()));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn only_ssh_signatures_count() {
        let commit = |sig: &str| format!("tree 1\nauthor a\ncommitter c\n{sig}\nmessage gpgsig -----BEGIN SSH SIGNATURE-----\n");
        assert!(ssh_signed(&commit("gpgsig -----BEGIN SSH SIGNATURE-----\n U1NIU0lH\n -----END SSH SIGNATURE-----\n")));
        assert!(!ssh_signed(&commit("gpgsig -----BEGIN PGP SIGNATURE-----\n iQ\n -----END PGP SIGNATURE-----\n")));
        assert!(!ssh_signed(&commit("gpgsig -----BEGIN SSH SIGNATURE-----\ngpgsig-sha256 -----BEGIN PGP SIGNATURE-----\n")));
        assert!(!ssh_signed(&commit(""))); // unsigned; the message doesn't count
    }

    fn collection_with(files: &[(&str, &str)]) -> (Collection, PathBuf) {
        let dir = env::temp_dir().join(format!("furnish-catalog-{}", unique()));
        for (path, contents) in files {
            fs::create_dir_all(dir.join(path).parent().unwrap()).unwrap();
            fs::write(dir.join(path), contents).unwrap();
        }
        (Collection { name: "c".into(), dir: dir.clone(), managed: false }, dir)
    }

    #[test]
    fn invalid_metadata_is_rejected_before_stamping_the_app() {
        let (mut collection, dir) = collection_with(&[("apps/app.dsc.yaml", "")]);
        collection.managed = true;
        let path = dir.join("apps/app.dsc.yaml");
        for (input, expected) in [
            ("false", "app document"), ("metadata: false", "metadata must"), ("metadata: { furnish: 1 }", "metadata.furnish must"),
        ] {
            fs::write(&path, input).unwrap();
            assert!(collection.load_app(&path).err().unwrap().to_string().contains(expected));
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn resource_types_ignore_case_and_installer_checks_hold() {
        let app = |kind: &str, file: &str| format!("resources:\n  - name: x\n    type: {kind}\n    properties: {{ file: '{file}' }}\n");
        let (c, dir) = collection_with(&[
            ("apps/lower.dsc.yaml", &app("furnish/installer", "/etc/passwd")),
            ("apps/upper.dsc.yaml", &app("FURNISH/INSTALLER", "i/x.deb")),
            ("apps/case.dsc.yaml", "resources:\n  - name: x\n    type: Furnish/Installer\n    properties: { File: 'C:/x.msi' }\n"),
        ]);
        assert!(c.load_app(&dir.join("apps/lower.dsc.yaml")).is_err(), "absolute file accepted");
        let upper = c.load_app(&dir.join("apps/upper.dsc.yaml")).unwrap();
        assert_eq!(upper.doc["resources"][0]["type"], "Furnish/Installer");
        assert!(upper.doc["resources"][0]["properties"]["file"].as_str().unwrap().starts_with(&*dir.canonicalize().unwrap().to_string_lossy()));
        assert!(c.load_app(&dir.join("apps/case.dsc.yaml")).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn command_input_sources_are_resolved_inside_the_collection() {
        let app = |source: &str| format!("resources:\n  - name: import\n    type: Furnish/Command\n    properties: {{ source: '{source}', test: {{ command: [tool] }}, set: [tool, '%s'] }}\n");
        let (collection, dir) = collection_with(&[
            ("apps/valid.dsc.yaml", &app("files/profile.ovpn")),
            ("apps/outside.dsc.yaml", &app("../outside.ovpn")),
            ("apps/missing.dsc.yaml", &app("files/missing.ovpn")),
            ("files/profile.ovpn", "client"),
        ]);
        let valid = collection.load_app(&dir.join("apps/valid.dsc.yaml")).unwrap();
        let path = valid.doc["resources"][0]["properties"]["source"].as_str().unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "client");
        assert!(collection.load_app(&dir.join("apps/outside.dsc.yaml")).is_err());
        let missing = collection.load_app(&dir.join("apps/missing.dsc.yaml")).unwrap();
        assert_eq!(missing.missing_installers(&serde_json::Map::new()).iter().map(Path::new).collect::<Vec<_>>(), [Path::new("files/missing.ovpn")]);
        #[cfg(unix)]
        {
            let outside = tempfile::NamedTempFile::new().unwrap();
            std::os::unix::fs::symlink(outside.path(), dir.join("files/missing.ovpn")).unwrap();
            assert!(collection.load_app(&dir.join("apps/missing.dsc.yaml")).is_err());
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn any_resource_type_loads_unless_the_policy_narrows_them() {
        let vpn = "resources:\n  - name: vpn\n    type: Microsoft.Windows/Registry\n    properties: { keyPath: 'HKLM\\\\Software\\\\Acme' }\n";
        // Nested in a group: the policy and the installer checks apply there too.
        let group = "resources:\n  - name: g\n    type: Microsoft.DSC/Group\n    properties:\n      resources:\n        - name: tool\n          type: furnish/package\n          properties: { name: tool }\n        - name: cmd\n          type: Microsoft.DSC.Transitional/RunCommandOnSet\n          properties: { executable: sh }\n        - name: setup\n          type: Furnish/Installer\n          properties: { file: /etc/passwd }\n";
        let (c, dir) = collection_with(&[("apps/vpn.dsc.yaml", vpn), ("apps/group.dsc.yaml", group)]);
        let path = dir.join("apps/vpn.dsc.yaml");
        let app = c.load_app_allowing(&path, None).unwrap();
        assert_eq!(app.doc["resources"][0]["type"], "Microsoft.Windows/Registry");
        // Allowed types ignore case; furnish's own are always allowed.
        assert!(c.load_app_allowing(&path, Some(&["microsoft.windows/registry".into()])).is_ok());
        let err = c.load_app_allowing(&path, Some(&[])).err().unwrap().to_string();
        assert!(err.contains("type 'Microsoft.Windows/Registry' isn't allowed by your organization"), "{err}");
        let group = dir.join("apps/group.dsc.yaml");
        let err = c.load_app_allowing(&group, None).err().unwrap().to_string();
        assert!(err.contains("outside the collection"), "nested installer not checked: {err}");
        fs::write(&group, fs::read_to_string(&group).unwrap().replace("/etc/passwd", "i/x.deb")).unwrap();
        let app = c.load_app_allowing(&group, None).unwrap();
        assert_eq!(app.doc["resources"][0]["properties"]["resources"][0]["type"], "Furnish/Package");
        let narrowed = Some(&["Microsoft.DSC/Group".to_string()][..]);
        let err = c.load_app_allowing(&group, narrowed).err().unwrap().to_string();
        assert!(err.contains("RunCommandOnSet' isn't allowed"), "nested type not narrowed: {err}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn removal_steps_get_the_checks_resources_get() {
        let app = |steps: &str| format!(
            "metadata:\n  furnish:\n    uninstall: {steps}\nresources:\n  - name: f\n    type: Microsoft.Filesystem.File/Content\n    properties: {{ path: /tmp/x }}\n"
        );
        let (c, dir) = collection_with(&[
            ("apps/ok.dsc.yaml", &app("[{ name: rm, type: Microsoft.DSC.Transitional/RunCommandOnSet, properties: { executable: rm } }, { name: u, type: furnish/installer, properties: { file: i/x.deb, _exist: false } }]")),
            ("apps/outside.dsc.yaml", &app("[{ name: u, type: Furnish/Installer, properties: { file: /etc/passwd } }]")),
            ("apps/empty.dsc.yaml", &app("[]")),
            ("apps/text.dsc.yaml", &app("rm -f /tmp/x")),
        ]);
        let ok = c.load_app(&dir.join("apps/ok.dsc.yaml")).unwrap();
        let steps = &ok.doc["metadata"]["furnish"]["uninstall"];
        assert_eq!(steps[1]["type"], "Furnish/Installer");
        assert!(steps[1]["properties"]["file"].as_str().unwrap().starts_with(&*dir.canonicalize().unwrap().to_string_lossy()));
        let err = |file: &str| format!("{:#}", c.load_app(&dir.join(file)).err().unwrap());
        assert!(err("apps/outside.dsc.yaml").contains("outside the collection"));
        assert!(err("apps/empty.dsc.yaml").contains("a list of DSC resources"));
        assert!(err("apps/text.dsc.yaml").contains("a list of DSC resources"));
        let narrowed = Some(&["Microsoft.Filesystem.File/Content".to_string()][..]);
        let err = format!("{:#}", c.load_app_allowing(&dir.join("apps/ok.dsc.yaml"), narrowed).err().unwrap());
        assert!(err.contains("RunCommandOnSet' isn't allowed"), "{err}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn installer_symlinks_out_of_the_collection_are_refused() {
        let yaml = "resources:\n  - name: x\n    type: Furnish/Installer\n    properties: { file: i/x.deb }\n";
        let (c, dir) = collection_with(&[("apps/a.dsc.yaml", yaml)]);
        fs::create_dir_all(dir.join("i")).unwrap();
        std::os::unix::fs::symlink("/etc/passwd", dir.join("i/x.deb")).unwrap();
        assert!(c.load_app(&dir.join("apps/a.dsc.yaml")).is_err(), "symlink out of the collection accepted");
        // Dangling: not "missing, checked out later".
        fs::remove_file(dir.join("i/x.deb")).unwrap();
        std::os::unix::fs::symlink("/nonexistent/x.deb", dir.join("i/x.deb")).unwrap();
        assert!(c.load_app(&dir.join("apps/a.dsc.yaml")).is_err(), "dangling symlink accepted");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn description_images_stay_inside_the_collection() {
        let (collection, dir) = collection_with(&[("images/profile.png", "picture")]);
        assert!(collection.description_image("./images/profile.png").is_some());
        for invalid in ["../images/profile.png", ".git/config", "images", "missing.png"] {
            assert!(collection.description_image(invalid).is_none());
        }
        #[cfg(unix)]
        {
            let outside = tempfile::NamedTempFile::new().unwrap();
            std::os::unix::fs::symlink(outside.path(), dir.join("images/link.png")).unwrap();
            assert!(collection.description_image("images/link.png").is_none());
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn dark_icons_load_beside_the_icons() {
        let (c, dir) = collection_with(&[
            ("collection.yaml", "icon: icons/c.svg\niconDark: icons/c-dark.svg\n"),
            ("apps/a.dsc.yaml", "metadata:\n  furnish:\n    icon: icons/a.svg\n    iconDark: icons/a-dark.svg\nresources: []\n"),
            ("apps/b.dsc.yaml", "metadata:\n  furnish:\n    icon: icons/a.svg\n    iconDark: icons/missing.svg\nresources: []\n"),
            ("kits/k.yaml", "icon: icons/c.svg\niconDark: icons/k-dark.svg\nitems:\n  - app: a\n"),
            ("icons/a.svg", "<svg/>"),
            ("icons/a-dark.svg", "<svg/>"),
            ("icons/c.svg", "<svg/>"),
            ("icons/c-dark.svg", "<svg/>"),
            ("icons/k-dark.svg", "<svg/>"),
        ]);
        let file = |name: &str| Some(dir.join("icons").join(name).canonicalize().unwrap());
        let a = c.load_app(&dir.join("apps/a.dsc.yaml")).unwrap();
        assert_eq!((a.icon, a.icon_dark), (file("a.svg"), file("a-dark.svg")));
        // A missing dark icon is dropped like a missing icon (furnish check names it).
        assert_eq!(c.load_app(&dir.join("apps/b.dsc.yaml")).unwrap().icon_dark, None);
        let k = c.load_kit(&dir.join("kits/k.yaml"), &["a".to_string()]).unwrap();
        assert_eq!((k.icon, k.icon_dark), (file("c.svg"), file("k-dark.svg")));
        assert_eq!((c.icon(), c.icon_dark()), (file("c.svg"), file("c-dark.svg")));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_chosen_version_stays_until_choosing_is_turned_off_a_fallback_until_the_app_changes() {
        let chosen = crate::versions::Held { commit: "c1".into(), chosen: true, ..Default::default() };
        assert!(hold_applies(&chosen, Some("changed"), true), "a collection change moved a chosen version");
        assert!(!hold_applies(&chosen, None, false), "kept while choosing is turned off");
        let fallback = crate::versions::Held { commit: "c1".into(), failed: "b2".into(), ..Default::default() };
        assert!(hold_applies(&fallback, Some("b2"), true));
        assert!(!hold_applies(&fallback, Some("b3"), true), "kept after the collection changed the app");
    }

    #[test]
    fn versions_older_than_the_maximum_age_are_left_out() {
        let day = 86_400;
        assert!(!older_than(0, 0, 1000 * day), "0 means any age");
        assert!(!older_than(7, 5 * day, 10 * day));
        assert!(!older_than(7, 3 * day, 10 * day), "exactly 7 days older");
        assert!(older_than(7, 3 * day - 1, 10 * day));
    }

    #[test]
    fn a_version_states_the_versions_its_file_accepts() {
        let file: Value = serde_yaml::from_str("resources:\n  - {name: a, type: T/X}\n  - {name: b, type: Furnish/Package, properties: {name: b, versions: ['>=2', '1.5']}}\n").unwrap();
        assert_eq!(crate::required::describe(&stated_versions(&file)), crate::required::describe(&crate::required::parse(&serde_json::json!([">=2", "1.5"])).unwrap()));
        assert!(stated_versions(&serde_json::json!({"resources": [{"name": "a"}]})).is_empty());
    }

    /// A collection repository with the history of one app: (message, contents or None to delete, days ago).
    fn app_history(steps: &[(&str, Option<&str>, i64)]) -> (Collection, PathBuf, Vec<String>) {
        let dir = env::temp_dir().join(format!("furnish-history-{}", unique()));
        fs::create_dir_all(dir.join("apps")).unwrap();
        git(Some(&dir), &["init", "-q"]).unwrap();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        let mut commits = Vec::new();
        for (message, contents, days_ago) in steps {
            match contents {
                Some(c) => fs::write(dir.join("apps/a.dsc.yaml"), c).unwrap(),
                None => fs::remove_file(dir.join("apps/a.dsc.yaml")).unwrap(),
            }
            fs::write(dir.join("notes.txt"), message).unwrap();
            git(Some(&dir), &["add", "-A"]).unwrap();
            let date = format!("@{} +0000", now - days_ago * 86_400);
            let status = git_command()
                .current_dir(&dir)
                .env("GIT_COMMITTER_DATE", &date)
                .env("GIT_AUTHOR_DATE", &date)
                .args(["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", message])
                .status()
                .unwrap();
            assert!(status.success());
            commits.push(git(Some(&dir), &["rev-parse", "HEAD"]).unwrap());
        }
        let collection = Collection { name: format!("history-test-{}", unique()), dir: dir.clone(), managed: false };
        (collection, dir, commits)
    }

    #[test]
    fn directory_source_snapshot_uses_only_files_from_the_chosen_commit() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir_all(repo.join("apps")).unwrap();
        fs::create_dir_all(repo.join("docs/chapter")).unwrap();
        let app = "resources: [{name: Docs, type: Furnish/Directory, properties: {path: ~/docs, source: docs/}}]";
        fs::write(repo.join("apps/docs.dsc.yaml"), app).unwrap();
        fs::write(repo.join("docs/chapter/page"), "original").unwrap();
        git(Some(&repo), &["init", "-q"]).unwrap();
        git(Some(&repo), &["add", "."]).unwrap();
        git(Some(&repo), &["-c","user.name=Test","-c","user.email=test@example.org","-c","commit.gpgsign=false","commit","-qm","docs"]).unwrap();
        let collection = Collection { name: "team".into(), dir: repo.clone(), managed: false };
        let commit = collection.head().unwrap();
        fs::write(repo.join("docs/chapter/page"), "local edit").unwrap();
        fs::write(repo.join("docs/local-only"), "local").unwrap();
        let snapshot = tmp.path().join("snapshot");
        collection.extract(&commit, "apps/docs.dsc.yaml", &serde_json::Map::new(), &snapshot).unwrap();
        assert_eq!(fs::read(snapshot.join("docs/chapter/page")).unwrap(), b"original");
        assert!(!snapshot.join("docs/local-only").exists());
    }

    #[test]
    fn held_packages_restore_scripts_from_their_own_commit() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("checkout");
        fs::create_dir_all(repo.join("apps")).unwrap();
        fs::create_dir_all(repo.join("managers")).unwrap();
        let collection = Collection { name: "org".into(), dir: repo.clone(), managed: true };
        let relative = "apps/tool.dsc.yaml";
        let app = "resources:\n  - { name: tool, type: Furnish/Package, properties: { name: tool, packageManager: local } }";
        fs::write(repo.join(relative), app).unwrap();
        fs::write(repo.join("apps/other.dsc.yaml"), app).unwrap();
        fs::write(repo.join("collection.yaml"),
            "packageManagers:\n  local: {script: 'managers/tool[1].sh'}\n  windows: {script: managers/tool.ps1}\n").unwrap();
        fs::write(repo.join("managers/tool[1].sh"), "old Unix").unwrap();
        fs::write(repo.join("managers/tool.ps1"), "old Windows").unwrap();
        git(Some(&repo), &["init", "-q"]).unwrap();
        let commit = |message: &str| {
            git(Some(&repo), &["add", "."]).unwrap();
            git(Some(&repo), &["-c", "user.name=Test", "-c", "user.email=test@example.org", "-c", "commit.gpgsign=false", "commit", "-qm", message]).unwrap();
            collection.head().unwrap()
        };
        let old = commit("first manager");
        fs::write(repo.join("managers/tool[1].sh"), "new Unix").unwrap();
        let new = commit("updated manager without changing app or declarations");
        assert_ne!(collection.manager_blobs(&old).unwrap(), collection.manager_blobs(&new).unwrap());
        let snapshot = version_dir(temp.path(), "org", &old);
        let values = serde_json::Map::new();
        collection.extract(&old, "apps/other.dsc.yaml", &values, &snapshot).unwrap();
        collection.extract(&old, relative, &values, &snapshot).unwrap();
        assert!(snapshot.join("apps/other.dsc.yaml").is_file(), "apps share the snapshot");
        assert_eq!(fs::read_to_string(snapshot.join("managers/tool.ps1")).unwrap(), "old Windows");

        // A complete service snapshot may be read-only. Leave even its timestamps alone.
        let ps = snapshot.join("managers/tool.ps1");
        let timestamp = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        fs::File::options().write(true).open(&ps).unwrap().set_modified(timestamp).unwrap();
        collection.extract(&old, relative, &values, &snapshot).unwrap();
        assert_eq!(fs::metadata(&ps).unwrap().modified().unwrap(), timestamp);

        // A hold created before scripts were extracted. Loading it reuses its files and
        // reads its missing script at the recorded commit, even though the checkout moved on.
        fs::remove_file(snapshot.join("managers/tool[1].sh")).unwrap();
        let app = collection.load_app(&repo.join(relative)).unwrap();
        let held = crate::versions::Held { commit: old, failed: collection.blob("HEAD", relative).unwrap(), ..Default::default() };
        let record = crate::versions::Record { held: Some(held.clone()), ..Default::default() };
        let records = [(BTreeMap::from([(app.key(), record)]), temp.path().to_path_buf())];
        assert_eq!(collection.with_hold(app, &records).held, Some(held));
        assert_eq!(fs::read_to_string(snapshot.join("managers/tool[1].sh")).unwrap(), "old Unix");
        assert_eq!(fs::read_to_string(repo.join("managers/tool[1].sh")).unwrap(), "new Unix");
        assert_eq!(collection.head().unwrap(), new);
    }

    #[test]
    fn an_apps_versions_are_the_commits_that_changed_its_file() {
        let v = |n: &str| format!("resources:\n  - {{name: a, type: Furnish/Package, properties: {{name: a, versions: ['>={n}']}}}}\n");
        let (c, dir, commits) = app_history(&[
            ("one", Some(&v("1")), 30),
            ("gone", None, 20),
            ("back", Some(&v("2")), 10),
            ("notes only", Some(&v("2")), 9),
            ("three", Some(&v("3")), 1),
        ]);
        let app = c.load_app(&dir.join("apps/a.dsc.yaml")).unwrap();
        let history = app.history().unwrap();
        let listed: Vec<&str> = history.iter().map(|(v, _, _)| v.commit.as_str()).collect();
        // Newest first; neither the deletion nor the commit that left the file alone.
        assert_eq!(listed, [&commits[4], &commits[2], &commits[0]]);
        assert!(history[0].0.current && !history[1].0.current);
        assert_eq!(history[2].2, git(Some(&dir), &["rev-parse", &format!("{}:apps/a.dsc.yaml", commits[0])]).unwrap());

        // By day or by commit.
        let day = history[1].0.date.clone();
        assert_eq!(app.resolve_version(&day).unwrap().commit, commits[2]);
        assert_eq!(app.resolve_version(&commits[0][..8]).unwrap().commit, commits[0]);
        assert_eq!(app.resolve_version(&commits[0][..8].to_uppercase()).unwrap().commit, commits[0]);
        assert!(app.resolve_version(&commits[1][..8]).is_err(), "the deletion is no version");
        assert!(app.resolve_version("abc").is_err(), "too short to be a commit");
        assert!(app.resolve_version("1999-01-01").unwrap_err().to_string().contains("no version from 1999-01-01"));

        // Each with the versions its file states.
        let contents = c.blobs(&history.iter().map(|(_, _, b)| b.as_str()).collect::<Vec<_>>());
        let stated = |i: usize| crate::required::describe(&stated_versions(&serde_yaml::from_slice(&contents[&history[i].2]).unwrap()));
        assert_eq!((stated(0), stated(2)), ("3 or later".to_string(), "1 or later".to_string()));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn two_versions_on_one_day_need_the_commit() {
        let (c, dir, commits) = app_history(&[("one", Some("resources: []\n"), 2), ("two", Some("resources: [] # 2\n"), 2), ("three", Some("resources: [] # 3\n"), 0)]);
        let app = c.load_app(&dir.join("apps/a.dsc.yaml")).unwrap();
        let day = app.history().unwrap()[1].0.date.clone();
        let err = app.resolve_version(&day).unwrap_err().to_string();
        assert!(err.contains("2 versions from") && err.contains(&commits[0][..7]) && err.contains(&commits[1][..7]), "{err}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn resources_here_leave_out_those_skipped_here() {
        let other = if cfg!(windows) { "Linux" } else { "Windows" };
        let app = format!(
            "parameters:\n  vpn:\n    type: bool\n    defaultValue: false\nresources:\n\
             - {{name: elsewhere, type: T/X, condition: \"[equals(context().os.family, '{other}')]\"}}\n\
             - {{name: profile, type: T/X, condition: \"[parameters('vpn')]\"}}\n\
             - {{name: here, type: T/X}}\n"
        );
        let (c, dir) = collection_with(&[("apps/a.dsc.yaml", &app)]);
        let a = c.load_app(&dir.join("apps/a.dsc.yaml")).unwrap();
        let names: Vec<Value> = a.resources_here().iter().map(|r| r["name"].clone()).collect();
        assert_eq!(names, vec![Value::from("here")]);
        fs::remove_dir_all(dir).unwrap();
    }
}
