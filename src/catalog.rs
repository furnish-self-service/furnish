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

pub struct App {
    pub id: String,
    pub collection: String,
    pub description: String,
    pub categories: Vec<String>,
    /// Optional icon image (`metadata.furnish.icon`, relative to the collection root).
    pub icon: Option<PathBuf>,
    /// The app's DSC configuration file.
    pub path: PathBuf,
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
    Ok(Collection { name: name.to_string(), dir })
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

    /// Resolve a repo-relative path; `None` if missing or pointing outside the repo.
    fn file_in_repo(&self, relative: &str) -> Option<PathBuf> {
        let path = self.dir.join(relative).canonicalize().ok()?;
        path.starts_with(self.dir.canonicalize().ok()?).then_some(path)
    }

    fn load_app(&self, path: &Path) -> Result<App> {
        let yaml: serde_yaml::Value = serde_yaml::from_str(&fs::read_to_string(path)?)?;
        let mut doc: Value = serde_json::to_value(yaml)?;
        if doc.get("$schema").is_none() {
            // DSC requires it; don't make every app author repeat it.
            doc["$schema"] = DSC_SCHEMA.into();
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
            doc,
        })
    }
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

    #[test]
    fn rejects_collection_names_that_escape_the_data_dir() {
        for name in ["../evil", "a/b", "..", ".hidden", ""] {
            assert!(add("https://example.com/x.git", name).is_err(), "{name} accepted");
        }
    }
}
