//! Package-manager scripts belong to one admin-preset collection and one revision of its checkout.
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{catalog, package_names, platform};

pub(crate) const SOURCE: &str = "furnishManagerSource";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Definition {
    pub script: String,
    pub interpreter: Option<String>,
    pub requires_command: Option<String>,
}

impl Definition {
    pub(crate) fn check(&self, id: &str) -> Result<()> {
        ensure!(
            package_names::valid_manager_id(id) && !platform::supported_manager(id),
            "invalid or built-in package manager name '{id}'"
        );
        let path = Path::new(&self.script);
        ensure!(
            !self.script.is_empty()
                && !self.script.contains(['\\', ':'])
                && !self.script.chars().any(char::is_control)
                && path
                    .components()
                    .all(|p| matches!(p, Component::Normal(n) if !n.eq_ignore_ascii_case(".git"))),
            "package manager '{id}' needs a script path inside the collection"
        );
        ensure!(
            self.interpreter.is_some() || matches!(extension(path).as_str(), "sh" | "ps1"),
            "package manager '{id}' needs an interpreter for scripts other than .sh or .ps1"
        );
        for (field, command) in [("interpreter", &self.interpreter), ("requiresCommand", &self.requires_command)] {
            let Some(command) = command else { continue };
            ensure!(
                !command.is_empty()
                    && command.bytes().all(|c| c.is_ascii_alphanumeric() || b"._+-".contains(&c))
                    && !matches!(command.as_str(), "." | ".."),
                "invalid {field} for package manager '{id}'"
            );
        }
        Ok(())
    }

    pub(crate) fn supports_platform(&self, windows: bool) -> bool {
        self.interpreter.is_some() || extension(Path::new(&self.script)) == if windows { "ps1" } else { "sh" }
    }

    fn path(&self, root: &Path) -> Result<PathBuf> {
        let path = root.join(&self.script);
        let real = path
            .canonicalize()
            .with_context(|| format!("package manager script '{}' is missing", self.script))?;
        ensure!(
            real.starts_with(root) && real.is_file(),
            "package manager script '{}' must be a file inside the collection",
            self.script
        );
        // Revisions are extracted as regular files. A symlink would run different content
        // after extraction, and could later be retargeted outside the checkout.
        ensure!(real == path, "package manager script '{}' must not traverse a symlink", self.script);
        Ok(real)
    }
}

fn extension(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase()
}

pub(crate) fn check_files(dir: &Path) -> Result<()> {
    let root = dir.canonicalize()?;
    for def in crate::options::package_managers(dir)?.values() {
        def.path(&root)?;
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    collection: String,
    dir: PathBuf,
}

fn check_selectors(resources: &Value, managers: &BTreeMap<String, Definition>) -> Result<()> {
    for resource in resources.as_array().into_iter().flatten() {
        let props = &resource["properties"];
        if resource["type"]
            .as_str()
            .is_some_and(|t| t.eq_ignore_ascii_case(catalog::PACKAGE_TYPE))
        {
            let manager = props["packageManager"].as_str().filter(|id| !id.starts_with('['));
            let aliases = props["namesByManager"]
                .as_object()
                .into_iter()
                .flat_map(|m| m.keys().map(String::as_str));
            for id in manager.into_iter().chain(aliases) {
                ensure!(
                    platform::supported_manager(id) || managers.contains_key(id),
                    "package manager '{id}' is not built in or declared under packageManagers in collection.yaml"
                );
            }
        }
        if let Some(children) = props.get("resources") {
            check_selectors(children, managers)?;
        }
    }
    Ok(())
}

pub(crate) fn attach(collection: &catalog::Collection, doc: &mut Value) -> Result<()> {
    ensure!(doc["metadata"]["furnish"].get(SOURCE).is_none(), "{SOURCE} is reserved for Furnish");
    let managers = crate::options::package_managers(&collection.dir)?;
    check_selectors(&doc["resources"], &managers)?;
    if let Some(steps) = doc.pointer("/metadata/furnish/uninstall") {
        check_selectors(steps, &managers)?;
    }
    if collection.managed && !managers.is_empty() {
        doc["metadata"]["furnish"][SOURCE] = serde_json::to_value(Source {
            collection: collection.name.clone(),
            dir: collection.dir.canonicalize()?,
        })?;
    }
    Ok(())
}

#[derive(Debug)]
pub(crate) struct Script {
    pub(crate) path: PathBuf,
    pub(crate) interpreter: Option<PathBuf>,
}

pub(crate) fn available(doc: &Value) -> Result<BTreeMap<String, Script>> {
    paths(&doc["metadata"]["furnish"][SOURCE], false)
}

/// DSC gives resources their per-step metadata on stdin. Only a current, policy-selected
/// checkout or that collection's revision directory can supply an executable script.
pub(crate) fn active(props: &Value) -> Result<BTreeMap<String, Script>> {
    paths(&props["_metadata"][SOURCE], true)
}

fn paths(source: &Value, verify_checkout: bool) -> Result<BTreeMap<String, Script>> {
    if source.is_null() {
        return Ok(BTreeMap::new());
    }
    let Source { collection, dir } = serde_json::from_value(source.clone())?;
    if verify_checkout {
        let current = catalog::get(&collection)?;
        ensure!(current.managed, "script package managers require an admin-preset collection");
        let in_versions = |root: PathBuf| {
            root.join("versions")
                .join(&collection)
                .canonicalize()
                .is_ok_and(|base| dir.parent() == Some(base.as_path()))
                && dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(".preview-") || (matches!(n.len(), 40 | 64) && n.bytes().all(|c| c.is_ascii_hexdigit())))
        };
        ensure!(
            dir == current.dir.canonicalize()? || in_versions(catalog::data_dir()) || in_versions(catalog::system_dir()),
            "package manager source does not belong to the preset collection '{collection}'"
        );
    }
    let mut available = BTreeMap::new();
    for (id, def) in crate::options::package_managers(&dir)? {
        let file = def.path(&dir)?;
        if !def.supports_platform(cfg!(windows)) || def.requires_command.as_deref().is_some_and(|c| which::which(c).is_err()) {
            continue;
        }
        let interpreter = match def.interpreter {
            Some(program) => {
                let Ok(path) = which::which(&program) else { continue };
                if cfg!(windows) && !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("exe")) {
                    continue;
                }
                let service =
                    crate::service::in_service() || std::env::var_os("FURNISH_SERVICE").as_deref() == Some(std::ffi::OsStr::new("1"));
                if service && !crate::service::admin_only(&path) {
                    continue;
                }
                Some(path)
            }
            None => None,
        };
        available.insert(id, Script { path: file, interpreter });
    }
    Ok(available)
}

/// Overwrite resource metadata with the scope of the document Furnish loaded. Collections
/// select a manager by name. They never supply a backend executable in package properties.
pub(crate) fn stamp(doc: &mut Value) {
    fn visit(resources: &mut Value, source: &Value, service_profile: bool) {
        for r in resources.as_array_mut().into_iter().flatten() {
            if r["type"].as_str().is_some_and(|t| t.eq_ignore_ascii_case(catalog::PACKAGE_TYPE)) {
                if let Some(props) = r["properties"].as_object_mut() {
                    props.remove("_metadata");
                }
                if service_profile {
                    r["metadata"][crate::nixos_apps::SERVICE_PROFILE] = true.into();
                } else if let Some(metadata) = r["metadata"].as_object_mut() {
                    metadata.remove(crate::nixos_apps::SERVICE_PROFILE);
                }
                if source.is_null() {
                    if let Some(meta) = r["metadata"].as_object_mut() {
                        meta.remove(SOURCE);
                    }
                } else {
                    r["metadata"][SOURCE] = source.clone();
                }
            }
            if let Some(children) = r["properties"].get_mut("resources") {
                visit(children, source, service_profile);
            }
        }
    }
    let source = doc["metadata"]["furnish"][SOURCE].clone();
    let shared = doc["metadata"]["furnish"][crate::nixos_apps::SERVICE_PROFILE] == true;
    visit(&mut doc["resources"], &source, shared);
}
