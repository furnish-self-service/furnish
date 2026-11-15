//! `furnish check`: validate a collection before people install from it. Loads every file the way
//! furnish does, checks what loading alone doesn't (installer files present, dependencies and kit
//! items that exist, expressions that evaluate, resources for every system), and previews which
//! resources run on Linux, macOS and Windows. Runs nothing.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;
use serde_json::{Map, Value};

use crate::catalog::{App, Collection};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub severity: Severity,
    pub message: String,
}

/// What `check` found about one file.
#[derive(Debug, Clone)]
pub struct Item {
    /// The app or kit, or `collection.yaml`.
    pub name: String,
    /// Kit, app or the collection's options.
    pub kind: &'static str,
    pub findings: Vec<Finding>,
    /// For apps: the resources that run per system, e.g. ("Windows", ["GIMP (winget)"]).
    pub systems: Vec<(&'static str, Vec<String>)>,
}

/// The systems an app can be for, as `context().os.family` names them.
pub const SYSTEMS: [&str; 3] = ["Linux", "macOS", "Windows"];
const RESOURCE_TYPES: [&str; 2] = ["Furnish/Package", "Furnish/Installer"];
const INSTALLER_TYPES: [&str; 6] = ["deb", "rpm", "pkg", "dmg", "msi", "exe"];

fn error(message: impl Into<String>) -> Finding {
    Finding { severity: Severity::Error, message: message.into() }
}

fn warning(message: impl Into<String>) -> Finding {
    Finding { severity: Severity::Warning, message: message.into() }
}

/// Check the collection in `dir` (a checkout of its repository).
pub fn collection(dir: &Path) -> Result<Vec<Item>> {
    let dir = dir.canonicalize()?;
    let name = dir.file_name().map_or_else(|| "collection".into(), |n| n.to_string_lossy().into_owned());
    let collection = Collection { name: name.clone(), dir: dir.clone(), managed: false };
    let mut items = Vec::new();

    let mut options = Item { name: "collection.yaml".into(), kind: "options", findings: vec![], systems: vec![] };
    let shared = match crate::options::collection_declarations(&dir) {
        Ok(shared) => shared,
        Err(err) => {
            options.findings.push(error(format!("{err:#}")));
            BTreeMap::new()
        }
    };
    if let Some(icon) = yaml_string(&dir.join("collection.yaml"), "icon")
        && !dir.join(&icon).is_file()
    {
        options.findings.push(error(format!("icon '{icon}' is not in the collection")));
    }
    if dir.join("collection.yaml").exists() {
        items.push(options);
    }

    // Values to evaluate expressions with: defaults, and the first choice for open questions.
    let mut values = Map::new();
    for r in crate::options::resolve_collection(&name, &shared) {
        if let Some(v) = r.value.clone().or_else(|| r.choices.first().map(|c| c.value.clone())) {
            values.insert(r.name, v);
        }
    }

    let mut apps = Vec::new();
    for path in files(&dir.join("apps"), ".dsc.yaml") {
        let id = file_stem(&path, ".dsc.yaml");
        match collection.load_app(&path) {
            Ok(app) => apps.push(app),
            Err(err) => items.push(Item { name: id, kind: "app", findings: vec![error(format!("{err:#}"))], systems: vec![] }),
        }
    }
    if apps.is_empty() && items.iter().all(|i| i.kind != "app") {
        items.push(Item { name: name.clone(), kind: "collection", findings: vec![error("no apps (apps/<name>.dsc.yaml)")], systems: vec![] });
    }
    let ids: Vec<String> = apps.iter().map(|a| a.id.clone()).collect();
    for app in &apps {
        items.push(check_app(app, &dir, &values, &apps));
    }

    for path in files(&dir.join("kits"), ".yaml") {
        let id = file_stem(&path, ".yaml");
        let mut item = Item { name: id, kind: "kit", findings: vec![], systems: vec![] };
        match collection.load_kit(&path, &ids) {
            Err(err) => item.findings.push(error(format!("{err:#}"))),
            // load_kit drops an icon it can't find.
            Ok(kit) => {
                if let Some(icon) = yaml_string(&path, "icon").filter(|_| kit.icon.is_none()) {
                    item.findings.push(error(format!("icon '{icon}' is not in the collection")));
                }
            }
        }
        items.push(item);
    }
    Ok(items)
}

fn check_app(app: &App, dir: &Path, collection_values: &Map<String, Value>, apps: &[App]) -> Item {
    let mut findings = Vec::new();
    let meta = &app.doc["metadata"]["furnish"];
    if app.description.is_empty() {
        findings.push(warning("no description (metadata.furnish.description)"));
    }
    if app.categories.is_empty() {
        findings.push(warning("no categories (metadata.furnish.categories)"));
    }
    if let Some(icon) = meta["icon"].as_str()
        && app.icon.is_none()
    {
        findings.push(error(format!("icon '{icon}' is not in the collection")));
    }

    let mut values = collection_values.clone();
    values.extend(crate::options::values(&app.options()));

    // DSC reads the document as furnish would run it.
    if let Err(err) = crate::engine::validate(&app.doc_for(&values)) {
        findings.push(error(format!("not a valid DSC document: {err:#}")));
    }

    let resources = app.doc["resources"].as_array().cloned().unwrap_or_default();
    if resources.is_empty() {
        findings.push(error("no resources"));
    }
    for r in &resources {
        let name = r["name"].as_str().unwrap_or("?");
        let kind = r["type"].as_str().unwrap_or_default();
        if !RESOURCE_TYPES.contains(&kind) {
            findings.push(warning(format!("{name}: {kind} is not a furnish resource; it runs only where DSC finds it")));
        }
        if kind == "Furnish/Package" && r["properties"]["name"].as_str().is_none_or(str::is_empty) {
            findings.push(error(format!("{name}: Furnish/Package needs properties.name")));
        }
        if kind == "Furnish/Installer" {
            match r["properties"]["file"].as_str() {
                None => findings.push(error(format!("{name}: Furnish/Installer needs properties.file"))),
                Some(file) => {
                    let path = PathBuf::from(file);
                    let extension = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                    if !INSTALLER_TYPES.contains(&extension.as_str()) {
                        findings.push(error(format!("{name}: {file}: not an installer furnish knows (.deb, .rpm, .pkg, .dmg, .msi, .exe)")));
                    }
                    // load_app made the path absolute; it may not be checked out (blobs_on_demand).
                    if !path.is_file() && !in_repository(dir, &path) {
                        findings.push(error(format!("{name}: {} is not in the collection", relative(dir, &path))));
                    }
                }
            }
        }
    }

    let mut systems = Vec::new();
    for system in SYSTEMS {
        let mut runs = Vec::new();
        for r in &resources {
            let name = r["name"].as_str().unwrap_or("?").to_string();
            match r["condition"].as_str() {
                None => runs.push(name),
                Some(condition) => match crate::options::condition_holds(&on_system(condition, system), &values) {
                    Ok(true) => runs.push(name),
                    Ok(false) => {}
                    Err(err) => {
                        findings.push(error(format!("{name}: condition {condition}: {err:#}")));
                        break;
                    }
                },
            }
        }
        systems.push((system, runs));
    }
    if systems.iter().all(|(_, runs)| runs.is_empty()) {
        findings.push(warning("no resource applies on Linux, macOS or Windows: furnish hides the app everywhere"));
    }

    if let Value::String(expr) = &meta["recommended"]
        && let Err(err) = crate::options::condition_holds(expr, &values)
    {
        findings.push(error(format!("recommended: {expr}: {err:#}")));
    }

    for dep in &app.dependencies {
        if !apps.iter().any(|a| a.id == dep.app) {
            findings.push(error(format!("depends on {}, which is not an app of the collection", dep.app)));
        }
        if let Some(condition) = &dep.condition
            && let Err(err) = crate::options::condition_holds(condition, &values)
        {
            findings.push(error(format!("dependency {}: condition {condition}: {err:#}", dep.app)));
        }
    }
    if let Some(cycle) = cycle(app, apps) {
        findings.push(error(format!("dependencies form a cycle: {}", cycle.join(" → "))));
    }

    Item { name: app.id.clone(), kind: "app", findings, systems }
}

/// The condition as it evaluates on another system: `context().os.family` replaced by its name.
/// ponytail: textual; conditions on other context (os.version, architecture) use this computer's.
fn on_system(condition: &str, system: &str) -> String {
    condition.replace("context().os.family", &format!("'{system}'"))
}

fn cycle(app: &App, apps: &[App]) -> Option<Vec<String>> {
    fn visit(id: &str, apps: &[App], path: &mut Vec<String>) -> Option<Vec<String>> {
        if let Some(start) = path.iter().position(|p| p == id) {
            let mut cycle = path[start..].to_vec();
            cycle.push(id.to_string());
            return Some(cycle);
        }
        let app = apps.iter().find(|a| a.id == id)?;
        path.push(id.to_string());
        for dep in &app.dependencies {
            if let Some(cycle) = visit(&dep.app, apps, path) {
                return Some(cycle);
            }
        }
        path.pop();
        None
    }
    visit(&app.id, apps, &mut Vec::new()).filter(|c| c.first() == Some(&app.id))
}

fn files(dir: &Path, suffix: &str) -> Vec<PathBuf> {
    let mut files: Vec<_> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.to_string_lossy().ends_with(suffix))
        .collect();
    files.sort();
    files
}

fn file_stem(path: &Path, suffix: &str) -> String {
    path.file_name().unwrap_or_default().to_string_lossy().trim_end_matches(suffix).to_string()
}

fn relative(dir: &Path, path: &Path) -> String {
    path.strip_prefix(dir).unwrap_or(path).to_string_lossy().into_owned()
}

/// Whether git has the file, also when the checkout leaves it out (sparse, see blobs_on_demand).
fn in_repository(dir: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(dir) else { return false };
    crate::catalog::git_command()
        .arg("-C")
        .arg(dir)
        .args(["ls-files", "--error-unmatch", "--"])
        .arg(relative)
        .output()
        .is_ok_and(|o| o.status.success())
}

fn yaml_string(path: &Path, key: &str) -> Option<String> {
    let yaml: serde_yaml::Value = serde_yaml::from_str(&fs::read_to_string(path).ok()?).ok()?;
    yaml.get(key)?.as_str().map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditions_evaluate_per_system() {
        let windows_only = "[equals(context().os.family, 'Windows')]";
        assert_eq!(on_system(windows_only, "Linux"), "[equals('Linux', 'Windows')]");
        let values = Map::new();
        assert!(!crate::options::condition_holds(&on_system(windows_only, "Linux"), &values).unwrap());
        assert!(crate::options::condition_holds(&on_system(windows_only, "Windows"), &values).unwrap());
    }
}
