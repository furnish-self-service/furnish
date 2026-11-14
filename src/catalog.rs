//! App collections: git repositories with one DSC configuration document per app in `apps/`.

use std::{
    env, fs,
    path::{Path, PathBuf},
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

/// Default collection name: last URL path segment without `.git`.
pub fn name_from_url(url: &str) -> String {
    let last = url.trim_end_matches('/').rsplit(['/', ':']).next().unwrap_or(url);
    last.trim_end_matches(".git").to_string()
}

/// Add a collection the user chose; the admin policy may forbid that.
pub fn add(url: &str, name: &str) -> Result<Collection> {
    let policy = crate::policy::load().collections;
    if !policy.users_can_add {
        bail!("your organization manages the collections; adding others isn't allowed");
    }
    if policy.preset.iter().any(|p| p.name == name) {
        bail!("'{name}' is the name of a collection your organization provides");
    }
    clone(url, name, false)
}

/// Clone the admin's preset collections this user doesn't have yet. Returns problems as messages
/// (e.g. offline), so a missing preset doesn't stop furnish.
pub fn ensure_presets() -> Vec<String> {
    let presets = crate::policy::load().collections.preset;
    if crate::service::enabled() {
        // The service sets them up; ask it only if one is missing.
        if presets.iter().all(|p| preset_dir().join(&p.name).join(".git").exists()) {
            return vec![];
        }
        return crate::service::ensure_presets().unwrap_or_else(|err| vec![format!("{err:#}")]);
    }
    let mut problems = Vec::new();
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
    git(None, &["clone", "--quiet", "--no-checkout", url, &dir.to_string_lossy()])?;
    let collection = Collection { name: name.to_string(), dir, managed };
    let checkout = || -> Result<()> {
        collection.verify("HEAD")?;
        git(Some(&collection.dir), &["reset", "--quiet", "--hard"])?;
        collection.fetch_lfs_files()
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

    /// Fast-forward to the latest upstream commit. Returns (old, new) revision.
    pub fn update(&self) -> Result<(String, String)> {
        if self.managed && crate::service::enabled() {
            return crate::service::update(&self.name);
        }
        let old = self.revision()?;
        git(Some(&self.dir), &["fetch", "--quiet"])?;
        self.verify("@{upstream}")?;
        git(Some(&self.dir), &["merge", "--quiet", "--ff-only", "@{upstream}"])?;
        self.fetch_lfs_files()?;
        Ok((old, self.revision()?))
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
            .with_context(|| format!("collection '{}': the latest commit isn't signed by a key your organization trusts", self.name))?;
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
        paths
            .iter()
            .filter_map(|path| match self.load_app(path) {
                Ok(app) => Some(app),
                Err(err) => {
                    eprintln!("warning: skipping {}: {err:#}", path.display());
                    None
                }
            })
            .collect()
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

    /// Collections may store installers with Git LFS; without it they'd hold pointer files.
    // ponytail: downloads every LFS file; per-OS `lfs.fetchinclude` if collections get large.
    fn fetch_lfs_files(&self) -> Result<()> {
        let uses_lfs = fs::read_to_string(self.dir.join(".gitattributes")).is_ok_and(|a| a.contains("filter=lfs"));
        if !uses_lfs {
            return Ok(());
        }
        git(Some(&self.dir), &["lfs", "version"])
            .map_err(|_| anyhow::anyhow!("collection '{}' stores files with Git LFS, but git-lfs is not installed", self.name))?;
        git(Some(&self.dir), &["lfs", "install", "--local"])?;
        git(Some(&self.dir), &["lfs", "pull"])?;
        Ok(())
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
                let resolved = self
                    .file_in_repo(file)
                    .with_context(|| format!("installer '{file}' is missing or outside the collection"))?;
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
        })
    }
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

    /// Data directory of whoever installs this app (the user, or the furnish service), which
    /// records the option values it was installed with.
    fn state_dir(&self) -> PathBuf {
        // Inside the service, data_dir() is the system directory too.
        if self.remote() { system_dir() } else { data_dir() }
    }

    /// Installed through the furnish service.
    fn remote(&self) -> bool {
        self.managed && crate::service::enabled()
    }

    /// Values for status, updates and uninstall: those it was installed with, else the current ones.
    pub fn installed_values(&self) -> serde_json::Map<String, Value> {
        crate::options::installed_with(&self.state_dir(), &self.key()).unwrap_or_else(|| self.current_values())
    }

    /// Installed with option values that differ from the current ones (a reinstall applies them).
    pub fn options_differ(&self) -> bool {
        let Some(installed) = crate::options::installed_with(&self.state_dir(), &self.key()) else { return false };
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
        let values = self.installed_values();
        if self.remote() {
            return crate::service::run(&crate::service::Request::Install { app: self.key(), values }, report);
        }
        self.install_with(&values, report)
    }

    pub fn install_with(&self, values: &serde_json::Map<String, Value>, report: impl FnMut(crate::engine::Progress)) -> Result<()> {
        crate::engine::run(&self.doc_for(values), false, report)?;
        crate::options::remember(&self.state_dir(), &self.key(), values)
    }

    pub fn uninstall(&self, report: impl FnMut(crate::engine::Progress)) -> Result<()> {
        if self.remote() {
            return crate::service::run(&crate::service::Request::Uninstall { app: self.key() }, report);
        }
        crate::engine::run(&self.doc_for(&self.installed_values()), true, report)?;
        crate::options::forget(&self.state_dir(), &self.key())
    }

    /// Apply changed options: uninstall with the old values, install with the current ones.
    pub fn reinstall(&self, report: impl FnMut(crate::engine::Progress)) -> Result<()> {
        let values = self.current_values();
        if self.remote() {
            return crate::service::run(&crate::service::Request::Reinstall { app: self.key(), values }, report);
        }
        self.reinstall_with(&values, report)
    }

    pub fn reinstall_with(&self, values: &serde_json::Map<String, Value>, mut report: impl FnMut(crate::engine::Progress)) -> Result<()> {
        crate::engine::run(&self.doc_for(&self.installed_values()), true, &mut report)?;
        crate::options::forget(&self.state_dir(), &self.key())?;
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

fn git(dir: Option<&Path>, args: &[&str]) -> Result<String> {
    let _working = crate::load::activity(); // load-aware updates ignore furnish's own downloads
    let mut cmd = Command::new("git");
    if let Some(dir) = dir {
        cmd.arg("-C").arg(dir);
        // The service's collections belong to root/SYSTEM; git refuses other users' repositories otherwise.
        if dir.starts_with(system_dir()) {
            cmd.args(["-c", "safe.directory=*"]);
        }
    }
    let output = cmd.args(args).output().context("failed to run git (is it installed?)")?;
    if !output.status.success() {
        bail!("git {} failed: {}", args.join(" "), String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
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
            assert!(add("https://example.com/x.git", name).is_err(), "{name} accepted");
        }
    }
}
