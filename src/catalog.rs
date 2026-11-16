//! App collections: git repositories with one DSC configuration document per app in `apps/`.

use std::{
    collections::BTreeMap,
    env, fs,
    io::Write,
    path::{Component, Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use serde_json::Value;

const DSC_SCHEMA: &str = "https://aka.ms/dsc/schemas/v3/bundled/config/document.json";

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
    /// Only for dependencies: a DSC expression over the app's options, e.g.
    /// "[parameters('userInstall')]"; the dependency applies only when it is true.
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
}

#[derive(Clone)]
pub struct App {
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
    /// Held at this earlier version (a commit), because the collection's current one failed;
    /// `doc` is that version's (see `versions`).
    pub held: Option<String>,
}

/// Which version of an app to install.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Which {
    /// The version furnish shows: the collection's, or the one the app is held at.
    #[default]
    Shown,
    /// The collection's version, also for a held app.
    Current,
    /// The earlier version to fall back to (see `App::previous_version`).
    Previous,
}

/// An earlier version of an app.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Version {
    pub commit: String,
    /// The day of the commit, e.g. 2026-11-02.
    pub date: String,
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
        // If users may not add collections, only the admin's count (others stay on disk, unused).
        .filter(|c| (policy.users_can_add || c.managed) && (!separate || !c.managed))
        // With the service, a preset's name belongs to the service's copy.
        .filter(|c| !separate || !preset_name(&c.name))
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
    presets.iter().any(|p| p.name == name && origin_url(dir).is_ok_and(|u| u == p.url))
}

fn origin_url(dir: &Path) -> Result<String> {
    git(Some(dir), &["config", "--get", "remote.origin.url"])
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
    check_name(name)?;
    let policy = crate::policy::load().collections;
    if !policy.users_can_add {
        bail!("your organization manages the collections; adding others isn't allowed");
    }
    if policy.preset.iter().any(|p| p.name == name) {
        bail!("'{name}' is the name of a collection your organization provides");
    }
    if let Some((user, password)) = credentials {
        crate::auth::store(url, user, password)?;
    }
    clone(url, name, false, false).inspect_err(|err| {
        if credentials.is_some() && err.is::<crate::auth::CredentialsRequired>() {
            crate::auth::forget(url); // wrong ones: don't keep them
        }
    })
}

/// Add an organization's furnish address (`furnish.myorg.tld`, see `endpoint`): sign in if
/// needed (`show` gets the sign-in URL), then add each of its collections. Returns the new ones.
pub fn add_endpoint(origin: &str, show: impl Fn(&str)) -> Result<Vec<Collection>> {
    let policy = crate::policy::load_native();
    let preset = policy.endpoints.iter().filter_map(|e| crate::endpoint::origin(e)).any(|o| o == origin);
    if !policy.collections.users_can_add && !preset {
        bail!("your organization manages the collections; adding others isn't allowed");
    }
    let description = crate::endpoint::discover(origin)?;
    if crate::endpoint::token(origin, &description).is_err() {
        crate::endpoint::login(origin, &description, show)?;
    }
    let presets = crate::policy::load().collections.preset;
    let mut added = Vec::new();
    for c in &description.collections {
        check_name(&c.name)?;
        if presets.iter().any(|p| p.name == c.name && p.url != c.url) {
            bail!("'{}' is the name of a collection your organization provides", c.name);
        }
        let dir = collections_dir().join(&c.name);
        if dir.join(".git").exists() {
            match origin_url(&dir) {
                Ok(url) if url == c.url => continue, // already there
                _ => bail!("a collection named '{}' already exists", c.name),
            }
        }
        added.push(clone(&c.url, &c.name, preset, false)?);
    }
    Ok(added)
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
        match clone(&preset.url, &preset.name, true, true) {
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
fn check_name(name: &str) -> Result<()> {
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
fn clone(url: &str, name: &str, managed: bool, replace: bool) -> Result<Collection> {
    check_name(name)?;
    fs::create_dir_all(collections_dir().join(".partial"))?;
    let _lock = lock(&collections_dir(), name)?;
    let dir = collections_dir().join(name);
    if dir.exists() {
        if !replace {
            bail!("a collection named '{name}' already exists");
        }
        if origin_url(&dir).is_ok_and(|u| u == url) {
            return Ok(Collection { name: name.to_string(), dir, managed }); // another furnish just did
        }
    }
    let settings = crate::settings::for_collection(name).settings;
    let depth = format!("--depth={}", settings.checkout_depth);
    let mut args = vec!["clone", "--quiet", "--no-checkout"];
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
        // One commit, resolved once: what is verified is what gets checked out.
        let head = git(Some(&collection.dir), &["rev-parse", "--verify", "--end-of-options", "HEAD^{commit}"])?;
        collection.verify(&head)?;
        collection.prepare_checkout(&settings, &head, true)?;
        // A commit id from rev-parse: no option (and git before 2.44 rejects --end-of-options here).
        git(Some(&collection.dir), &["reset", "--quiet", "--hard", &head])?;
        if dir.exists() {
            let old = collections_dir().join(".partial").join(format!("{name}+{}", unique()));
            fs::rename(&dir, &old)?;
            remove_repository(&old);
        }
        Ok(fs::rename(&staging, &dir)?)
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

fn remember_remote_head(file: &Path, name: &str, commit: &str) {
    let mut heads = read_remote_heads(file);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    heads.insert(name.into(), serde_json::json!({ "commit": commit, "checkedAt": now }));
    // Losing this only loses the indicator until the next look.
    let _ = fs::write(file, serde_json::to_vec_pretty(&heads).unwrap_or_default());
}

fn remote_head_differs(file: &Path, name: &str, head: &str) -> bool {
    read_remote_heads(file).get(name).and_then(|h| h["commit"].as_str()).is_some_and(|c| c != head)
}

/// The commit the server's branch points to, without downloading anything (`git ls-remote`).
fn server_head(dir: &Path) -> Result<String> {
    let upstream = git(Some(dir), &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"])?;
    let (remote, branch) = upstream.split_once('/').with_context(|| format!("no remote branch in '{upstream}'"))?;
    let out = git(Some(dir), &["ls-remote", "--end-of-options", remote, &format!("refs/heads/{branch}")])?;
    out.split_whitespace().next().map(str::to_string).with_context(|| format!("the server has no branch {branch}"))
}

pub fn remove(name: &str) -> Result<()> {
    let collection = get(name)?;
    if collection.managed {
        bail!("'{name}' is provided by your organization and can't be removed");
    }
    if let Ok(url) = collection.url() {
        crate::auth::forget(&url);
    }
    fs::remove_dir_all(&collection.dir)?;
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

    /// Ask the server for its newest commit (cheap: nothing is downloaded) and remember it.
    /// Offline or failing: an error, and what was remembered stays.
    pub fn check_server(&self) -> Result<()> {
        let commit = server_head(&self.dir)?;
        remember_remote_head(&remote_heads_file(), &self.name, &commit);
        Ok(())
    }

    /// The server had a commit this copy hasn't checked out, when furnish last looked.
    pub fn update_available(&self) -> bool {
        self.head().is_ok_and(|head| remote_head_differs(&remote_heads_file(), &self.name, &head))
    }

    /// Fast-forward to the latest upstream commit. Returns (old, new) revision.
    pub fn update(&self) -> Result<(String, String)> {
        let result = self.update_unreported();
        if result.is_ok()
            && let Ok(head) = self.head()
        {
            remember_remote_head(&remote_heads_file(), &self.name, &head);
        }
        crate::reports::record(crate::reports::Event {
            event: "collectionUpdate",
            outcome: if result.is_ok() { "succeeded" } else { "failed" },
            collection: Some(self.name.clone()),
            version: result.as_ref().ok().map(|(_, new)| new.clone()),
            error: result.as_ref().err().map(|e| format!("{e:#}")),
            ..Default::default()
        });
        result
    }

    fn update_unreported(&self) -> Result<(String, String)> {
        if self.managed && crate::service::enabled() {
            return crate::service::update(&self.name);
        }
        let _lock = self.lock()?;
        let old = self.revision()?;
        let settings = crate::settings::for_collection(&self.name).settings;
        let depth = format!("--depth={}", settings.checkout_depth);
        let mut fetch = vec!["fetch", "--quiet"];
        if settings.checkout_depth > 0 {
            fetch.push(&depth);
        } else if git(Some(&self.dir), &["rev-parse", "--is-shallow-repository"])? == "true" {
            fetch.push("--unshallow");
        }
        git(Some(&self.dir), &fetch)?;
        // One commit, resolved once: what is verified is what gets checked out.
        let new = git(Some(&self.dir), &["rev-parse", "--verify", "--end-of-options", "@{upstream}^{commit}"])?;
        // Fetched, not applied yet: available until checked out (it may still fail the checks).
        remember_remote_head(&remote_heads_file(), &self.name, &new);
        self.verify(&new)?;
        self.require_descendant(&new)?;
        self.prepare_checkout(&settings, &new, false)?;
        // ponytail: reset, not merge: in a shallow history git may not see that the new commit
        // follows the old one. Collections are furnish's copies, nobody commits to them.
        git(Some(&self.dir), &["reset", "--quiet", "--hard", &new])?; // a commit id, see clone
        Ok((old, self.revision()?))
    }

    /// The collection's lock (see `lock`).
    fn lock(&self) -> Result<Option<fs::File>> {
        lock(self.dir.parent().unwrap_or(&self.dir), &self.name)
    }

    /// With trusted keys: `new` must follow the current commit. Otherwise whoever controls the
    /// server could move the collection back to an older signed commit, with its known problems.
    fn require_descendant(&self, new: &str) -> Result<()> {
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
            git(Some(&self.dir), &["fetch", "--quiet", "--unshallow"])?;
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
        paths
            .iter()
            .filter_map(|path| match self.load_app_allowing(path, allowed.as_deref()) {
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
        if self.blob("HEAD", &relative).as_deref() != Some(held.failed.as_str()) {
            return app; // the collection changed the app: the next install tries that version
        }
        let version = version_dir(dir, &self.name, &held.commit);
        let earlier = Collection { name: self.name.clone(), dir: version.clone(), managed: self.managed };
        match earlier.load_app(&version.join(&relative)) {
            Ok(earlier) => {
                app.doc = earlier.doc;
                app.dependencies = earlier.dependencies;
                app.held = Some(held.commit);
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

    /// The last `n` commits that changed the file, newest first.
    fn changes(&self, relative: &str, n: usize) -> Vec<(String, i64, String)> {
        let count = format!("-{n}");
        let out = git(Some(&self.dir), &["log", &count, "--no-renames", "--format=%H %ct %cs", "--", relative]).unwrap_or_default();
        out.lines().filter_map(parse_commit_line).collect()
    }

    /// Write files of `commit` to `into`: `collection.yaml`, the app's file, and the installer
    /// files it uses with these values.
    fn extract(&self, commit: &str, relative: &str, values: &serde_json::Map<String, Value>, into: &Path) -> Result<()> {
        let write = |file: &str| -> Result<()> {
            let target = into.join(file);
            fs::create_dir_all(target.parent().unwrap_or(into))?;
            // --filters: what a checkout writes, e.g. the file itself for one in Git LFS.
            let spec = format!("{commit}:{}", file.replace('\\', "/"));
            let contents = git_bytes(Some(&self.dir), &["cat-file", "--filters", "--end-of-options", &spec])?;
            Ok(fs::write(target, contents)?)
        };
        if self.blob(commit, "collection.yaml").is_some() {
            write("collection.yaml")?;
        }
        write(relative)?;
        let earlier = Collection { name: self.name.clone(), dir: into.to_path_buf(), managed: self.managed };
        for file in earlier.load_app(&into.join(relative))?.missing_installers(values) {
            write(&file)?;
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
        })
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
        let yaml: serde_yaml::Value = serde_yaml::from_str(&fs::read_to_string(path)?)?;
        let mut doc: Value = serde_json::to_value(yaml)?;
        if doc.get("$schema").is_none() {
            // DSC requires it; don't make every app author repeat it.
            doc["$schema"] = DSC_SCHEMA.into();
        }
        self.check_resources(&mut doc["resources"], allowed)?;
        // Removal steps are resources too: the same policy and installer checks.
        if let Some(steps) = doc.pointer_mut("/metadata/furnish/uninstall") {
            if !steps.as_array().is_some_and(|s| !s.is_empty() && s.iter().all(Value::is_object)) {
                bail!("metadata.furnish.uninstall: a list of DSC resources, like resources");
            }
            self.check_resources(steps, allowed).context("metadata.furnish.uninstall")?;
        }
        let meta = &doc["metadata"]["furnish"];
        let file_name = path.file_name().unwrap_or_default().to_string_lossy();
        Ok(App {
            id: file_name.trim_end_matches(".dsc.yaml").to_string(),
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
    /// (`properties.resources`): types the policy allows, and installer files in the collection.
    fn check_resources(&self, resources: &mut Value, allowed: Option<&[String]>) -> Result<()> {
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
                self.check_resources(&mut resource["properties"]["resources"], allowed)?;
            }
            if canonical != Some(&INSTALLER_TYPE) {
                continue;
            }
            // Windows passes properties as environment variables, whose names ignore case.
            let keys = resource["properties"].as_object().into_iter().flat_map(|p| p.keys());
            if let Some(key) = keys.into_iter().find(|k| k.eq_ignore_ascii_case("file") && *k != "file") {
                bail!("installer property '{key}': did you mean 'file'?");
            }
            // Installer files are given relative to the collection; resources need absolute paths.
            if let Some(file) = resource["properties"]["file"].as_str() {
                if file.chars().any(char::is_control) {
                    bail!("installer '{}' has control characters in its name", file.escape_debug());
                }
                let outside = || format!("installer '{file}' is outside the collection");
                let resolved = match fs::symlink_metadata(self.dir.join(file)) {
                    Ok(_) => self.file_in_repo(file).with_context(outside)?,
                    // Not checked out yet (blobs_on_demand): installing the app checks it out.
                    Err(_) => self.file_in_repo_later(file).with_context(outside)?,
                };
                resource["properties"]["file"] = resolved.to_string_lossy().into_owned().into();
            } else if !resource["properties"]["file"].is_null() {
                bail!("installer file must be a path in the collection");
            }
        }
        Ok(())
    }
}

/// The resource types the admin policy allows apps to use besides furnish's own; `None`: any.
pub(crate) fn allowed_resource_types() -> Option<Vec<String>> {
    crate::policy::load_native().allowed_resource_types
}

/// The resources furnish ships (see `engine::init`). Apps may use other DSC resources too.
pub const RESOURCE_TYPES: [&str; 2] = ["Furnish/Package", INSTALLER_TYPE];
const INSTALLER_TYPE: &str = "Furnish/Installer";

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

/// Windows canonical paths start with \\?\, which installers like msiexec reject.
fn plain_path(path: PathBuf) -> PathBuf {
    path.to_string_lossy().strip_prefix(r"\\?\").map(PathBuf::from).unwrap_or(path)
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
        match &self.doc["metadata"]["furnish"]["recommended"] {
            Value::Bool(b) => *b,
            Value::String(expr) => {
                let dir = &self.collection_dir;
                let shared = crate::options::collection_declarations(dir).unwrap_or_default();
                let mut values = crate::options::values(&crate::options::resolve_collection(&self.collection, &shared));
                values.extend(self.current_values());
                // An unanswered option makes the expression fail: not recommended (yet).
                crate::options::condition_holds(expr, &values).unwrap_or(false)
            }
            _ => false,
        }
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

    /// Installs into the user's account with these values: `metadata.furnish.perUser`, a DSC
    /// expression over the options (e.g. `[parameters('userInstall')]`) or `true`. Such installs
    /// never go through the furnish service: root or SYSTEM would install for the wrong account.
    pub fn per_user(&self, values: &serde_json::Map<String, Value>) -> bool {
        match &self.doc["metadata"]["furnish"]["perUser"] {
            Value::Bool(b) => *b,
            Value::String(expr) => crate::options::condition_holds(expr, values).unwrap_or_else(|err| {
                eprintln!("warning: {}: perUser: {err:#}", self.id);
                false
            }),
            _ => false,
        }
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
    fn recorded_values(&self) -> Option<serde_json::Map<String, Value>> {
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
        self.current_values().iter().any(|(k, v)| installed.get(k) != Some(v))
    }

    pub fn doc_for(&self, values: &serde_json::Map<String, Value>) -> Value {
        crate::options::apply(&self.doc, values)
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
                Some(c) => crate::options::condition_holds(c, values).unwrap_or_else(|err| {
                    eprintln!("warning: {}: dependency {} condition: {err:#}", self.id, d.app);
                    false
                }),
            })
            .cloned()
            .collect()
    }

    pub fn check(&self) -> Result<crate::engine::Check> {
        crate::engine::check(&self.doc_for(&self.installed_values()))
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
        let doc = self.doc_for(values);
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

    /// Install the current version, or the previous one (see `previous_version`). Reported.
    pub fn install_version(&self, which: Which, report: impl FnMut(crate::engine::Progress)) -> Result<()> {
        let event = match which {
            Which::Previous => "fallback",
            _ if self.recorded_values().is_some() => "update",
            _ => "install",
        };
        let version = match which {
            Which::Previous => self.previous_version().ok().map(|v| v.commit),
            Which::Shown if self.held.is_some() => self.held.clone(),
            _ => self.collection_handle().head().ok(),
        };
        crate::reports::operation(event, self, version, || self.install_values(self.installed_values(), which, report))
    }

    /// Install with these values: through the furnish service, or here.
    fn install_values(&self, values: serde_json::Map<String, Value>, which: Which, report: impl FnMut(crate::engine::Progress)) -> Result<()> {
        self.require_answers()?;
        if self.remote(&values) {
            return crate::service::run(&crate::service::Request::Install { app: self.key(), values, which }, report);
        }
        self.install_here(&values, which, report)
    }

    pub fn install_here(&self, values: &serde_json::Map<String, Value>, which: Which, report: impl FnMut(crate::engine::Progress)) -> Result<()> {
        let collection = self.collection_handle();
        let _lock = collection.lock()?;
        // The checkout may have changed since furnish verified it: only install a signed one.
        collection.verify(&collection.head()?)?;
        let result = match which {
            Which::Shown => self.install_with(values, report),
            Which::Current => self.current()?.install_with(values, report),
            Which::Previous => return self.install_previous_with(values, report),
        };
        // Remembered, so the app may go back to an earlier version (see install_previous_with).
        if let Err(err) = &result
            && !err.is::<crate::engine::Cancelled>()
            && let Some(blob) = collection.blob("HEAD", &self.relative_path())
            && let Err(record) = crate::versions::failed(&self.state_dir(values), &self.key(), &blob)
        {
            eprintln!("warning: {}: couldn't record the failure: {record:#}", self.key());
        }
        result
    }

    pub fn install_with(&self, values: &serde_json::Map<String, Value>, report: impl FnMut(crate::engine::Progress)) -> Result<()> {
        self.check_out_installers(values)?;
        let doc = self.doc_for(values);
        crate::engine::run(&doc, false, report)?;
        self.require_installed(&doc)?;
        let state = self.state_dir(values);
        crate::options::remember(&state, &self.key(), values)?;
        let collection = self.collection_handle();
        let held = self.held.as_ref().map(|commit| crate::versions::Held {
            commit: commit.clone(),
            failed: collection.blob("HEAD", &self.relative_path()).unwrap_or_default(),
        });
        let commit = match &self.held {
            Some(commit) => commit.clone(),
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

    /// Installer files (relative to the collection) the resources and removal steps that apply
    /// with these values use and that aren't on disk.
    fn missing_installers(&self, values: &serde_json::Map<String, Value>) -> Vec<String> {
        let mut files = Vec::new();
        let steps = crate::engine::removal_steps(&self.doc).into_iter().flatten();
        for resource in self.doc["resources"].as_array().into_iter().flatten().chain(steps) {
            let Some(file) = resource["properties"]["file"].as_str().filter(|_| resource["type"] == INSTALLER_TYPE) else {
                continue;
            };
            let applies = resource["condition"].as_str().is_none_or(|c| crate::options::condition_holds(c, values).unwrap_or(true));
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
        let commit = self.held.as_ref()?;
        let (commit, _, date) = self.collection_handle().commit_info(commit).ok()?;
        Some(Version { commit, date })
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
        if days > 0 && changes.first().is_some_and(|(_, newest, _)| newest - time > i64::from(days) * 86_400) {
            bail!("the previous version of {} is from {date}, more than {days} days older than the current one", self.id);
        }
        collection.verify(&commit)?;
        Ok(Version { commit, date })
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
        let dir = version_dir(&state, &self.collection, &version.commit);
        if !dir.join(&relative).exists() {
            let _ = fs::remove_dir_all(&dir);
            if let Err(err) = collection.extract(&version.commit, &relative, values, &dir) {
                let _ = fs::remove_dir_all(&dir);
                return Err(err.context(format!("getting the version from {}", version.date)));
            }
        }
        let earlier = Collection { name: self.collection.clone(), dir: dir.clone(), managed: self.managed }.load_app(&dir.join(&relative))?;
        let doc = earlier.doc_for(values);
        crate::engine::run(&doc, false, report)?;
        earlier.require_installed(&doc)?;
        crate::options::remember(&state, &self.key(), values)?;
        let failed = collection.blob("HEAD", &relative).unwrap_or_default();
        let held = crate::versions::Held { commit: version.commit.clone(), failed };
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
        self.remove_with(&values, report)?;
        crate::versions::forget(&self.state_dir(&values), &self.key())?;
        crate::options::forget(&self.state_dir(&values), &self.key())
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
        if self.remote(&old) && self.remote(&new) {
            return crate::service::run(&crate::service::Request::Reinstall { app: self.key(), values: new }, report);
        }
        if self.remote(&old) || self.remote(&new) {
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
        self.remove_with(&old, &mut report)?;
        crate::options::forget(&self.state_dir(&old), &self.key())?;
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

/// Installer files, which blobs_on_demand leaves out of the checkout until an app needs one.
const INSTALLERS: [&str; 6] = ["*.msi", "*.exe", "*.pkg", "*.dmg", "*.deb", "*.rpm"];

/// The git program: the system's, else the copy the Windows installer ships next to furnish
/// (MinGit with git-lfs, in `git\` beside furnish.exe).
pub fn git_command() -> Command {
    static BUNDLED: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    let bundled = BUNDLED.get_or_init(|| {
        let on_path = env::var_os("PATH").is_some_and(|p| env::split_paths(&p).any(|d| d.join("git.exe").is_file()));
        if !cfg!(windows) || on_path {
            return None;
        }
        let git = env::current_exe().ok()?.parent()?.join("git").join("cmd").join("git.exe");
        git.is_file().then_some(git)
    });
    // The service runs as root: git from the system, not from a folder on PATH that a user may own
    // (e.g. Homebrew's). NixOS has none in /usr/bin; there the service's PATH is the system's.
    if !cfg!(windows) && crate::service::in_service() && Path::new("/usr/bin/git").is_file() {
        return Command::new("/usr/bin/git");
    }
    let Some(git) = bundled else { return Command::new("git") };
    let mut cmd = Command::new(git);
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

fn git_bytes(dir: Option<&Path>, args: &[&str]) -> Result<Vec<u8>> {
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
    let network = matches!(
        args.first(),
        Some(&("clone" | "fetch" | "pull" | "lfs" | "ls-remote" | "reset" | "read-tree" | "cat-file"))
    );
    let url = if !network {
        None
    } else if args.first() == Some(&"clone") {
        match args.iter().position(|a| *a == "--end-of-options") {
            Some(end) => args.get(end + 1).map(|a| a.to_string()),
            None => args.iter().skip(1).find(|a| !a.starts_with('-')).map(|a| a.to_string()),
        }
    } else {
        dir.and_then(|d| {
            let out = git_command().arg("-C").arg(d).args(["config", "--get", "remote.origin.url"]).output().ok()?;
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
        })
    };
    if let Some(url) = &url {
        cmd.envs(crate::auth::git_env(url)?);
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
        let k = Kit { id: "k".into(), collection: "c".into(), name: None, description: file.description, categories: file.categories, icon: None, icon_dark: None, items: file.items };
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
        let look = || remember_remote_head(&file, "c", &server_head(&copy).unwrap());

        assert!(!remote_head_differs(&file, "c", &head()), "never looked");
        look();
        assert!(!remote_head_differs(&file, "c", &head()), "up to date");
        commit("two");
        look();
        assert!(remote_head_differs(&file, "c", &head()), "the server moved on");
        assert!(!remote_head_differs(&file, "other", &head()), "per collection");
        // Offline: no answer, and what was seen before stays.
        fs::rename(&origin, dir.join("gone")).unwrap();
        assert!(server_head(&copy).is_err());
        assert!(remote_head_differs(&file, "c", &head()));
        fs::rename(dir.join("gone"), &origin).unwrap();
        // Applied: the indicator goes.
        git(Some(&copy), &["pull", "-q"]).unwrap();
        assert!(!remote_head_differs(&file, "c", &head()));
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
