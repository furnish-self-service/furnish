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
}

/// A curated set of apps from the same collection, installed together (`kits/<id>.yaml`).
pub struct Kit {
    pub id: String,
    pub collection: String,
    pub description: String,
    pub categories: Vec<String>,
    pub icon: Option<PathBuf>,
    pub items: Vec<KitItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KitItem {
    /// App id in the kit's collection.
    pub app: String,
    /// Always installed with the kit; users can't deselect it.
    #[serde(default)]
    pub required: bool,
    /// For optional items: selected when the user starts installing the kit.
    #[serde(default = "yes")]
    pub selected: bool,
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
    items: Vec<KitItem>,
}

impl Kit {
    /// The apps installing this kit starts with: required ones and optional ones selected by default.
    pub fn default_selection(&self) -> Vec<&str> {
        self.items.iter().filter(|i| i.required || i.selected).map(|i| i.app.as_str()).collect()
    }
}

pub struct App {
    pub id: String,
    pub collection: String,
    pub description: String,
    pub categories: Vec<String>,
    /// Optional icon image (`metadata.furnish.icon`, relative to the collection root).
    pub icon: Option<PathBuf>,
    /// The app's DSC configuration file.
    pub path: PathBuf,
    /// Apps of the same collection to install first (`metadata.furnish.requires`), e.g. Homebrew.
    pub requires: Vec<String>,
    pub doc: Value,
}

/// Data directory; `FURNISH_DATA_DIR` overrides the OS default.
pub fn data_dir() -> PathBuf {
    env::var_os("FURNISH_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::data_dir().expect("no data directory on this OS").join("furnish"))
}

fn collections_dir() -> PathBuf {
    data_dir().join("collections")
}

pub fn collections() -> Result<Vec<Collection>> {
    let dir = collections_dir();
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut collections: Vec<_> = fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join(".git").exists())
        .map(|e| Collection { name: e.file_name().to_string_lossy().into_owned(), dir: e.path() })
        .collect();
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

pub fn add(url: &str, name: &str) -> Result<Collection> {
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
    git(None, &["clone", "--quiet", url, &dir.to_string_lossy()])?;
    let collection = Collection { name: name.to_string(), dir };
    if let Err(err) = collection.fetch_lfs_files() {
        let _ = fs::remove_dir_all(&collection.dir); // no half-usable collection
        return Err(err);
    }
    Ok(collection)
}

pub fn remove(name: &str) -> Result<()> {
    let collection = get(name)?;
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
        let old = self.revision()?;
        git(Some(&self.dir), &["pull", "--quiet", "--ff-only"])?;
        self.fetch_lfs_files()?;
        Ok((old, self.revision()?))
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
            requires: meta["requires"]
                .as_array()
                .map(|r| r.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default(),
            doc,
        })
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
    let mut cmd = Command::new("git");
    if let Some(dir) = dir {
        cmd.arg("-C").arg(dir);
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
            items: items.iter().map(|(app, required)| KitItem { app: (*app).into(), required: *required, selected: true }).collect(),
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
        assert_eq!(k.default_selection(), ["a", "b"]);
    }

    #[test]
    fn rejects_collection_names_that_escape_the_data_dir() {
        for name in ["../evil", "a/b", "..", ".hidden", ""] {
            assert!(add("https://example.com/x.git", name).is_err(), "{name} accepted");
        }
    }
}
