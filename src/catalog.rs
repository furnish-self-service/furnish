//! App collections: git repositories with one DSC configuration document per app in `apps/`.

use std::{
    collections::BTreeMap,
    env, fs,
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
    pub description: String,
    pub categories: Vec<String>,
    pub icon: Option<PathBuf>,
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
    #[serde(default)]
    description: String,
    #[serde(default)]
    categories: Vec<String>,
    icon: Option<String>,
    items: Vec<Item>,
}

#[derive(Clone)]
pub struct App {
    pub id: String,
    pub collection: String,
    pub description: String,
    pub categories: Vec<String>,
    /// Optional icon image (`metadata.furnish.icon`, relative to the collection root).
    pub icon: Option<PathBuf>,
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
    let preset = |name: &str| policy.preset.iter().any(|p| p.name == name);
    let in_dir = |dir: PathBuf| -> Result<Vec<Collection>> {
        if !dir.exists() {
            return Ok(vec![]);
        }
        Ok(fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().join(".git").exists())
            .map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                Collection { managed: preset(&name), name, dir: e.path() }
            })
            .collect())
    };
    let mut collections: Vec<_> = in_dir(collections_dir())?
        .into_iter()
        // If users may not add collections, only the admin's count (others stay on disk, unused).
        .filter(|c| (policy.users_can_add || c.managed) && (preset_dir() == collections_dir() || !c.managed))
        .collect();
    if preset_dir() != collections_dir() {
        collections.extend(in_dir(preset_dir())?.into_iter().filter(|c| c.managed));
    }
    collections.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(collections)
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
    let spec = crate::policy::load().branding.logo?;
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
    clone(url, name, false).inspect_err(|err| {
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
    let mut added = Vec::new();
    for c in &description.collections {
        let dir = collections_dir().join(&c.name);
        if dir.join(".git").exists() {
            match git(Some(&dir), &["remote", "get-url", "origin"]) {
                Ok(url) if url == c.url => continue, // already there
                _ => bail!("a collection named '{}' already exists", c.name),
            }
        }
        added.push(clone(&c.url, &c.name, preset)?);
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
        // The service sets them up; ask it only if one is missing.
        if presets.iter().all(|p| preset_dir().join(&p.name).join(".git").exists()) {
            return problems;
        }
        problems.extend(crate::service::ensure_presets().unwrap_or_else(|err| vec![format!("{err:#}")]));
        return problems;
    }
    for preset in presets {
        let dir = collections_dir().join(&preset.name);
        if dir.join(".git").exists() {
            match git(Some(&dir), &["remote", "get-url", "origin"]) {
                Ok(url) if url == preset.url => {}
                _ => problems.push(format!("collection '{}' exists but doesn't come from {}", preset.name, preset.url)),
            }
            continue;
        }
        if let Err(err) = clone(&preset.url, &preset.name, true) {
            problems.push(format!("couldn't set up collection '{}': {err:#}", preset.name));
        }
    }
    problems
}

fn clone(url: &str, name: &str, managed: bool) -> Result<Collection> {
    // The name becomes a directory name: no separators, no `..`, no hidden dirs.
    let valid = !name.is_empty()
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    if !valid {
        bail!("invalid collection name '{name}': use letters, digits, '.', '_' and '-'");
    }
    let dir = collections_dir().join(name);
    if dir.exists() {
        bail!("a collection named '{name}' already exists");
    }
    fs::create_dir_all(collections_dir())?;
    let settings = crate::settings::for_collection(name).settings;
    let depth = format!("--depth={}", settings.checkout_depth);
    let mut args = vec!["clone", "--quiet", "--no-checkout"];
    if settings.blobs_on_demand {
        args.push("--filter=blob:none");
    }
    if settings.checkout_depth > 0 {
        args.push(&depth);
    }
    // Commits that left the history (see checkout_depth) are deleted at the next cleanup.
    args.extend(["--config=gc.reflogExpireUnreachable=now", "--config=gc.pruneExpire=now"]);
    let target = dir.to_string_lossy();
    args.extend([url, &target]);
    git(None, &args)?;
    let collection = Collection { name: name.to_string(), dir, managed };
    let checkout = || -> Result<()> {
        collection.verify("HEAD")?;
        collection.prepare_checkout(&settings, "HEAD", true)?;
        git(Some(&collection.dir), &["reset", "--quiet", "--hard"]).map(drop)
    };
    if let Err(err) = checkout() {
        let _ = fs::remove_dir_all(&collection.dir); // no half-usable collection
        return Err(err);
    }
    Ok(collection)
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

    /// The collection's avatar (`icon` in `collection.yaml`).
    pub fn icon(&self) -> Option<PathBuf> {
        self.file_in_repo(&crate::options::collection_icon(&self.dir)?)
    }

    /// Fast-forward to the latest upstream commit. Returns (old, new) revision.
    pub fn update(&self) -> Result<(String, String)> {
        if self.managed && crate::service::enabled() {
            return crate::service::update(&self.name);
        }
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
        self.verify("@{upstream}")?;
        self.prepare_checkout(&settings, "@{upstream}", false)?;
        // ponytail: reset, not merge: in a shallow history git may not see that the new commit
        // follows the old one. Collections are furnish's copies, nobody commits to them.
        git(Some(&self.dir), &["reset", "--quiet", "--hard", "@{upstream}"])?;
        Ok((old, self.revision()?))
    }

    /// Set up what checking out `rev` writes: git-lfs (so files it checks out are downloaded,
    /// not left as pointers), and with blobs_on_demand everything except installer files.
    fn prepare_checkout(&self, settings: &crate::settings::CollectionSettings, rev: &str, fresh: bool) -> Result<()> {
        let attributes = git(Some(&self.dir), &["cat-file", "-p", &format!("{rev}:.gitattributes")]).unwrap_or_default();
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
            let kept = if fresh { String::new() } else { git(Some(&self.dir), &[&["ls-files", "--"][..], &INSTALLERS].concat())? };
            let mut patterns = String::from("/*\n");
            for pattern in INSTALLERS {
                patterns += &format!("!{pattern}\n");
            }
            for file in kept.lines() {
                patterns += &format!("/{file}\n");
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
            patterns += &format!("/{}\n", file.replace('\\', "/"));
        }
        fs::write(&sparse, patterns)?;
        git(Some(&self.dir), &["read-tree", "-mu", "HEAD"])?;
        match files.iter().find(|f| !self.dir.join(f).is_file()) {
            Some(missing) => bail!("collection '{}' has no file {missing}", self.name),
            None => Ok(()),
        }
    }

    /// For preset collections, when the policy lists trusted keys: refuse a commit that isn't
    /// signed by one of them (SSH signatures). Whoever can push there controls what gets installed.
    fn verify(&self, rev: &str) -> Result<()> {
        let keys = crate::policy::load().collections.trusted_keys;
        if !self.managed || keys.is_empty() {
            return Ok(());
        }
        let signers = self.dir.join(".git").join("furnish-trusted-keys");
        let lines: String = keys.iter().map(|k| format!("* namespaces=\"git\" {}\n", k.trim())).collect();
        fs::write(&signers, lines)?;
        let config = format!("gpg.ssh.allowedSignersFile={}", signers.display());
        git(Some(&self.dir), &["-c", &config, "verify-commit", rev])
            .with_context(|| format!("collection '{}': commit {rev} isn't signed by a key your organization trusts", self.name))?;
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
        paths
            .iter()
            .filter_map(|path| match self.load_app(path) {
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
        git(Some(&self.dir), &["rev-parse", "--verify", "--quiet", &format!("{rev}:{}", relative.replace('\\', "/"))]).ok()
    }

    /// (commit, time, day) of `rev`.
    fn commit_info(&self, rev: &str) -> Result<(String, i64, String)> {
        let line = git(Some(&self.dir), &["log", "-1", "--format=%H %ct %cs", rev])?;
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
            let contents = git_bytes(Some(&self.dir), &["cat-file", "--filters", &format!("{commit}:{}", file.replace('\\', "/"))])?;
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

    fn load_kit(&self, path: &Path, app_ids: &[String]) -> Result<Kit> {
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
            description: file.description.trim().to_string(),
            categories: file.categories,
            icon: file.icon.as_deref().and_then(|p| self.file_in_repo(p)),
            items: file.items,
        })
    }

    /// Resolve a repo-relative path; `None` if missing or pointing outside the repo.
    fn file_in_repo(&self, relative: &str) -> Option<PathBuf> {
        let path = self.dir.join(relative).canonicalize().ok()?;
        if !path.starts_with(self.dir.canonicalize().ok()?) {
            return None;
        }
        // Windows canonical paths start with \\?\, which installers like msiexec reject.
        let plain = path.to_string_lossy().strip_prefix(r"\\?\").map(PathBuf::from);
        Some(plain.unwrap_or(path))
    }

    fn load_app(&self, path: &Path) -> Result<App> {
        let yaml: serde_yaml::Value = serde_yaml::from_str(&fs::read_to_string(path)?)?;
        let mut doc: Value = serde_json::to_value(yaml)?;
        if doc.get("$schema").is_none() {
            // DSC requires it; don't make every app author repeat it.
            doc["$schema"] = DSC_SCHEMA.into();
        }
        // Installer files are given relative to the collection; resources need absolute paths.
        for resource in doc["resources"].as_array_mut().into_iter().flatten() {
            if resource["type"] != "Furnish/Installer" {
                continue;
            }
            if let Some(file) = resource["properties"]["file"].as_str() {
                // Not checked out yet (blobs_on_demand): installing the app checks it out.
                let later = || Path::new(file).components().all(|c| matches!(c, Component::Normal(_))).then(|| self.dir.join(file));
                let resolved = self
                    .file_in_repo(file)
                    .or_else(later)
                    .with_context(|| format!("installer '{file}' is outside the collection"))?;
                resource["properties"]["file"] = resolved.to_string_lossy().into_owned().into();
            }
        }
        let meta = &doc["metadata"]["furnish"];
        let file_name = path.file_name().unwrap_or_default().to_string_lossy();
        Ok(App {
            id: file_name.trim_end_matches(".dsc.yaml").to_string(),
            collection: self.name.clone(),
            description: meta["description"].as_str().unwrap_or_default().trim().to_string(),
            categories: meta["categories"]
                .as_array()
                .map(|c| c.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default(),
            icon: meta["icon"].as_str().and_then(|p| self.file_in_repo(p)),
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
        crate::engine::plan(&self.doc_for(&self.installed_values()), uninstall)
    }

    /// Install or update, with the values it was installed with (else the current ones), and
    /// remember them.
    pub fn install(&self, report: impl FnMut(crate::engine::Progress)) -> Result<()> {
        self.install_values(self.installed_values(), Which::Shown, report)
    }

    /// Install the current version, or the previous one (see `previous_version`).
    pub fn install_version(&self, which: Which, report: impl FnMut(crate::engine::Progress)) -> Result<()> {
        self.install_values(self.installed_values(), which, report)
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
        match which {
            Which::Shown => self.install_with(values, report),
            Which::Current => self.current()?.install_with(values, report),
            Which::Previous => self.install_previous_with(values, report),
        }
    }

    pub fn install_with(&self, values: &serde_json::Map<String, Value>, report: impl FnMut(crate::engine::Progress)) -> Result<()> {
        self.check_out_installers(values)?;
        crate::engine::run(&self.doc_for(values), false, report)?;
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

    /// Installer files (relative to the collection) the resources that apply with these values
    /// use and that aren't on disk.
    fn missing_installers(&self, values: &serde_json::Map<String, Value>) -> Vec<String> {
        let mut files = Vec::new();
        for resource in self.doc["resources"].as_array().into_iter().flatten() {
            let Some(file) = resource["properties"]["file"].as_str().filter(|_| resource["type"] == "Furnish/Installer") else {
                continue;
            };
            let applies = resource["condition"].as_str().is_none_or(|c| crate::options::condition_holds(c, values).unwrap_or(true));
            if applies && !Path::new(file).is_file()
                && let Ok(relative) = Path::new(file).strip_prefix(&self.collection_dir)
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
    fn install_previous_with(&self, values: &serde_json::Map<String, Value>, report: impl FnMut(crate::engine::Progress)) -> Result<()> {
        let version = self.previous_version()?;
        let collection = self.collection_handle();
        let relative = self.relative_path();
        let state = self.state_dir(values);
        // ponytail: version folders stay after the app is uninstalled; prune them if they pile up.
        let dir = version_dir(&state, &self.collection, &version.commit);
        if !dir.join(&relative).exists() {
            let _ = fs::remove_dir_all(&dir);
            if let Err(err) = collection.extract(&version.commit, &relative, values, &dir) {
                let _ = fs::remove_dir_all(&dir);
                return Err(err.context(format!("getting the version from {}", version.date)));
            }
        }
        let earlier = Collection { name: self.collection.clone(), dir: dir.clone(), managed: self.managed }.load_app(&dir.join(&relative))?;
        crate::engine::run(&earlier.doc_for(values), false, report)?;
        crate::options::remember(&state, &self.key(), values)?;
        let failed = collection.blob("HEAD", &relative).unwrap_or_default();
        let held = crate::versions::Held { commit: version.commit.clone(), failed };
        crate::versions::installed(&state, &self.key(), &version.commit, Some(held))
    }

    /// Uninstall with the values it was installed with.
    pub fn uninstall(&self, report: impl FnMut(crate::engine::Progress)) -> Result<()> {
        let values = self.installed_values();
        if self.remote(&values) {
            return crate::service::run(&crate::service::Request::Uninstall { app: self.key() }, report);
        }
        crate::engine::run(&self.doc_for(&values), true, report)?;
        crate::versions::forget(&self.state_dir(&values), &self.key())?;
        crate::options::forget(&self.state_dir(&values), &self.key())
    }

    /// Apply changed options: uninstall with the old values, install with the current ones.
    /// Moving between a system-wide and a per-user install takes both the service and this process.
    pub fn reinstall(&self, mut report: impl FnMut(crate::engine::Progress)) -> Result<()> {
        self.require_answers()?;
        let (old, new) = (self.installed_values(), self.current_values());
        if self.remote(&old) && self.remote(&new) {
            return crate::service::run(&crate::service::Request::Reinstall { app: self.key(), values: new }, report);
        }
        if self.remote(&old) || self.remote(&new) {
            self.uninstall(&mut report)?;
            return self.install_values(new, Which::Shown, report);
        }
        self.reinstall_with(&new, report)
    }

    pub fn reinstall_with(&self, values: &serde_json::Map<String, Value>, mut report: impl FnMut(crate::engine::Progress)) -> Result<()> {
        let old = self.installed_values();
        crate::engine::run(&self.doc_for(&old), true, &mut report)?;
        crate::options::forget(&self.state_dir(&old), &self.key())?;
        self.install_with(values, report)
    }
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
        args.iter().skip(1).find(|a| !a.starts_with('-')).map(|a| a.to_string())
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
            description: String::new(),
            categories: vec![],
            icon: None,
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
        let k = Kit { id: "k".into(), collection: "c".into(), description: file.description, categories: file.categories, icon: None, items: file.items };
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
}
