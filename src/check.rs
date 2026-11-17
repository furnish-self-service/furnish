//! `furnish check`: validate a collection before people install from it. Loads every file the way
//! furnish does, checks what loading alone doesn't (installer files present, dependencies and kit
//! items that exist, expressions that evaluate, resources for every system), and previews which
//! resources run on Windows, macOS and Linux. Runs nothing.

use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;
use serde_json::{Map, Value};

use crate::catalog::{App, Collection, RESOURCE_TYPES};

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
    /// For translations: how much they cover, e.g. "17 of 19 apps, 3 of 3 kits".
    pub coverage: Option<String>,
}

/// The systems an app can be for, as `context().os.family` names them.
pub const SYSTEMS: [&str; 3] = ["Windows", "macOS", "Linux"];
const INSTALLER_TYPES: [&str; 6] = ["deb", "rpm", "pkg", "dmg", "msi", "exe"];

fn error(message: impl Into<String>) -> Finding {
    Finding { severity: Severity::Error, message: message.into() }
}

fn warning(message: impl Into<String>) -> Finding {
    Finding { severity: Severity::Warning, message: message.into() }
}

/// An `icon` or `iconDark` that names a file the collection doesn't have.
fn missing_icon(key: &str, path: &str) -> Finding {
    error(format!("{key} '{path}' is not in the collection"))
}

/// Check the collection in `dir` (a checkout of its repository).
pub fn collection(dir: &Path) -> Result<Vec<Item>> {
    let dir = dir.canonicalize()?;
    let name = dir.file_name().map_or_else(|| "collection".into(), |n| n.to_string_lossy().into_owned());
    let collection = Collection { name: name.clone(), dir: dir.clone(), managed: false };
    let mut items = Vec::new();

    let mut options = Item { name: "collection.yaml".into(), kind: "options", findings: vec![], systems: vec![], coverage: None };
    let shared = match crate::options::collection_declarations(&dir) {
        Ok(shared) => shared,
        Err(err) => {
            options.findings.push(error(format!("{err:#}")));
            BTreeMap::new()
        }
    };
    // Collection::icon is None for a file outside the collection (also through a symlink).
    for (key, found) in [("icon", collection.icon()), ("iconDark", collection.icon_dark())] {
        if let Some(icon) = yaml_string(&dir.join("collection.yaml"), key)
            && !found.is_some_and(|p| p.is_file())
        {
            options.findings.push(missing_icon(key, &icon));
        }
    }
    if let Some(language) = yaml_string(&dir.join("collection.yaml"), "language")
        && !crate::locale::valid_tag(&language)
    {
        options.findings.push(error(format!("language '{language}' is not a language tag such as en or pt-BR")));
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
    let mut kits = Vec::new();
    for path in files(&dir.join("apps"), ".dsc.yaml") {
        let id = file_stem(&path, ".dsc.yaml");
        match collection.load_app(&path) {
            Ok(app) => apps.push(app),
            Err(err) => items.push(Item { name: id, kind: "app", findings: vec![error(format!("{err:#}"))], systems: vec![], coverage: None }),
        }
    }
    if apps.is_empty() && items.iter().all(|i| i.kind != "app") {
        items.push(Item { name: name.clone(), kind: "collection", findings: vec![error("no apps (apps/<name>.dsc.yaml)")], systems: vec![], coverage: None });
    }
    let ids: Vec<String> = apps.iter().map(|a| a.id.clone()).collect();
    for app in &apps {
        items.push(check_app(app, &dir, &values, &apps));
    }

    for path in files(&dir.join("kits"), ".yaml") {
        let id = file_stem(&path, ".yaml");
        let mut item = Item { name: id, kind: "kit", findings: vec![], systems: vec![], coverage: None };
        match collection.load_kit(&path, &ids) {
            Err(err) => item.findings.push(error(format!("{err:#}"))),
            // load_kit drops an icon it can't find.
            Ok(kit) => {
                for (key, found) in [("icon", &kit.icon), ("iconDark", &kit.icon_dark)] {
                    if let Some(icon) = yaml_string(&path, key).filter(|_| found.is_none()) {
                        item.findings.push(missing_icon(key, &icon));
                    }
                }
                kits.push(kit);
            }
        }
        items.push(item);
    }
    items.extend(check_locales(&dir, &apps, &kits, &shared));
    Ok(items)
}

/// The translations (`locales/<lang>.yaml`): files that load, with texts for apps, kits, options
/// and categories the collection has. Missing translations are fine (see `locale`).
fn check_locales(dir: &Path, apps: &[App], kits: &[crate::catalog::Kit], shared: &BTreeMap<String, crate::options::Declaration>) -> Vec<Item> {
    let language = crate::options::collection_language(dir);
    let mut categories: Vec<&str> = apps.iter().flat_map(|a| &a.categories).chain(kits.iter().flat_map(|k| &k.categories)).map(String::as_str).collect();
    categories.sort();
    categories.dedup();
    // Options and their allowed values (as `allowedValues` keys write them).
    let mut parameters: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let declarations = shared.iter().map(|(n, d)| (n.clone(), d.clone())).chain(apps.iter().flat_map(|a| crate::options::app_declarations(&a.doc)));
    for (name, decl) in declarations {
        let values = parameters.entry(name).or_default();
        values.extend(decl.choices().iter().map(|c| crate::locale::value_key(&c.value)));
    }
    let mut items = Vec::new();
    for (tag, path) in crate::locale::locale_paths(dir) {
        let mut item = Item { name: format!("locales/{tag}.yaml"), kind: "translation", findings: vec![], systems: vec![], coverage: None };
        if !crate::locale::valid_tag(&tag) {
            item.findings.push(error(format!("'{tag}' is not a language tag such as de or pt-BR; name the file after the language")));
        }
        if tag.eq_ignore_ascii_case(&language) {
            item.findings.push(warning(format!(
                "{tag} is the language of the collection's own texts (language in collection.yaml): furnish shows those, not this file"
            )));
        }
        let file = match crate::locale::load_file(&path) {
            Ok(file) => file,
            Err(err) => {
                item.findings.push(error(format!("{err:#}")));
                items.push(item);
                continue;
            }
        };
        let mut unknown = |what: &str, name: &str, known: bool| {
            if !known {
                item.findings.push(warning(format!("{what} '{name}' is not in the collection")));
            }
        };
        for id in file.apps.keys() {
            unknown("app", id, apps.iter().any(|a| &a.id == id));
        }
        for id in file.kits.keys() {
            unknown("kit", id, kits.iter().any(|k| &k.id == id));
        }
        for key in file.categories.keys() {
            unknown("category", key, categories.contains(&key.as_str()));
        }
        for (name, texts) in &file.parameters {
            match parameters.get(name) {
                None => unknown("option", name, false),
                Some(values) => {
                    for value in texts.allowed_values.keys() {
                        unknown(&format!("option {name}: allowed value"), value, values.contains(value));
                    }
                }
            }
        }
        let n = |count: usize, of: usize, words: (&str, &str)| format!("{count} of {of} {}", if of == 1 { words.0 } else { words.1 });
        let texts = |t: Option<&crate::locale::Texts>| t.is_some_and(|t| t.name.is_some() || t.description.is_some());
        let parts = [
            n(apps.iter().filter(|a| texts(file.apps.get(&a.id))).count(), apps.len(), ("app", "apps")),
            n(kits.iter().filter(|k| texts(file.kits.get(&k.id))).count(), kits.len(), ("kit", "kits")),
            n(categories.iter().filter(|c| file.categories.contains_key(**c)).count(), categories.len(), ("category", "categories")),
            n(parameters.keys().filter(|p| file.parameters.get(*p).is_some_and(|t| t.label.is_some() || t.description.is_some())).count(), parameters.len(), ("option", "options")),
        ];
        item.coverage = Some(parts.join(", "));
        items.push(item);
    }
    items
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
    for (key, found) in [("icon", &app.icon), ("iconDark", &app.icon_dark)] {
        if let Some(icon) = meta[key].as_str()
            && found.is_none()
        {
            findings.push(missing_icon(key, icon));
        }
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
    check_resources(&resources, "", dir, &mut findings);
    let steps = crate::engine::removal_steps(&app.doc).cloned();
    match &steps {
        Some(steps) => {
            if let Some(removal) = crate::engine::removal_doc(&app.doc_for(&values))
                && let Err(err) = crate::engine::validate(&removal)
            {
                findings.push(error(format!("removal steps: not valid as a DSC document: {err:#}")));
            }
            check_resources(steps, "removal step ", dir, &mut findings);
            for r in steps {
                if let Some(condition) = r["condition"].as_str() {
                    for system in SYSTEMS {
                        if let Err(err) = crate::options::condition_holds(&on_system(condition, system), &values) {
                            let name = r["name"].as_str().unwrap_or("?");
                            findings.push(error(format!("removal step {name}: condition {condition}: {err:#}")));
                            break;
                        }
                    }
                }
            }
        }
        // Without removal steps, furnish sets each resource to `_exist: false` to uninstall.
        // Types DSC doesn't find here aren't judged (see the warning about them above).
        None => {
            let cant: Vec<String> = resources
                .iter()
                .filter(|r| matches!(crate::engine::removes(r["type"].as_str().unwrap_or_default()), Ok(Some(false))))
                .map(|r| format!("{} ({})", r["name"].as_str().unwrap_or("?"), r["type"].as_str().unwrap_or_default()))
                .collect();
            if !cant.is_empty() {
                findings.push(warning(format!(
                    "can't be uninstalled where these resources apply: {}. They can't remove what they set up, so furnish \
                     doesn't offer Uninstall there. Add removal steps (metadata.furnish.uninstall)",
                    cant.join(", ")
                )));
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
        findings.push(warning("no resource applies on Windows, macOS or Linux: furnish hides the app everywhere"));
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

    Item { name: app.id.clone(), kind: "app", findings, systems, coverage: None }
}

/// What loading doesn't check about each resource: that DSC finds its type here, and what
/// furnish's own types need. `label` goes before a resource's name, e.g. "removal step ".
fn check_resources(resources: &[Value], label: &str, dir: &Path, findings: &mut Vec<Finding>) {
    for r in resources {
        let name = format!("{label}{}", r["name"].as_str().unwrap_or("?"));
        let kind = r["type"].as_str().unwrap_or_default();
        // DSC matches resource types ignoring case, so `furnish/installer` is Furnish/Installer.
        let kind = RESOURCE_TYPES.iter().copied().find(|t| t.eq_ignore_ascii_case(kind)).unwrap_or(kind);
        // Another DSC resource may be installed where the app runs, just not here.
        if !RESOURCE_TYPES.contains(&kind) {
            match crate::engine::available(kind) {
                Ok(true) => {}
                Ok(false) => findings.push(warning(format!("{name}: DSC doesn't find the resource {kind} on this computer; the app needs it where it is installed"))),
                Err(err) => findings.push(error(format!("{name}: {err:#}"))),
            }
        }
        if kind == "Furnish/Package" && r["properties"]["name"].as_str().is_none_or(str::is_empty) {
            findings.push(error(format!("{name}: Furnish/Package needs properties.name")));
        }
        if kind == "Furnish/NixOS" && r["properties"]["name"].as_str().is_none_or(str::is_empty) {
            findings.push(error(format!("{name}: Furnish/NixOS needs properties.name")));
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
}

/// The condition as it evaluates on another system: `context().os.family` replaced by its name.
/// ponytail: textual; conditions on other context (os.version, architecture) use this computer's.
fn on_system(condition: &str, system: &str) -> String {
    condition.replace("context().os.family", &format!("'{system}'"))
}

/// A dependency cycle through `app`, e.g. [a, b, a]. Depth-first; each app is explored once (an app
/// fully explored without reaching `app` can't lead back to it), so diamonds stay linear.
fn cycle(app: &App, apps: &[App]) -> Option<Vec<String>> {
    fn visit<'a>(id: &'a str, target: &str, apps: &'a [App], path: &mut Vec<&'a str>, done: &mut HashSet<&'a str>) -> bool {
        if !path.is_empty() && id == target {
            return true;
        }
        if !done.insert(id) {
            return false;
        }
        let Some(app) = apps.iter().find(|a| a.id == id) else { return false };
        path.push(id);
        if app.dependencies.iter().any(|dep| visit(&dep.app, target, apps, path, done)) {
            return true;
        }
        path.pop();
        false
    }
    let mut path = Vec::new();
    if !visit(&app.id, &app.id, apps, &mut path, &mut HashSet::new()) {
        return None;
    }
    let mut cycle: Vec<String> = path.into_iter().map(String::from).collect();
    cycle.push(app.id.clone());
    Some(cycle)
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
    // The repository is the one being checked: its config must not run commands
    // (core.fsmonitor; hooks for good measure).
    crate::catalog::git_command()
        .arg("-C")
        .arg(dir)
        .args(["-c", "core.fsmonitor=false", "-c", "core.hooksPath=/dev/null"])
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

    fn app(id: &str, deps: &[String]) -> App {
        let dependencies = deps
            .iter()
            .map(|d| crate::catalog::Item { app: d.clone(), required: true, selected: true, condition: None })
            .collect();
        App {
            id: id.into(),
            collection: "c".into(),
            name: None,
            description: String::new(),
            categories: vec![],
            icon: None,
            icon_dark: None,
            path: PathBuf::new(),
            dependencies,
            collection_dir: PathBuf::new(),
            managed: false,
            doc: Value::Null,
            held: None,
        }
    }

    #[test]
    fn translations_warn_about_unknown_names_fail_on_wrong_types_and_report_coverage() {
        let dir = std::env::temp_dir().join(format!("furnish-check-locales-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("locales")).unwrap();
        fs::write(dir.join("collection.yaml"), "language: en\n").unwrap();
        fs::write(
            dir.join("locales/de.yaml"),
            "categories: { Office: Büro, Ghost: Geist }\napps: { mail: { description: E-Mail }, nope: { name: Nein } }\n\
             parameters: { role: { label: Rolle, allowedValues: { developer: Entwicklung, chef: Koch } } }\n",
        )
        .unwrap();
        fs::write(dir.join("locales/fr.yaml"), "apps: { mail: { description: [not, text] } }\n").unwrap();
        fs::write(dir.join("locales/en.yaml"), "").unwrap();
        fs::write(dir.join("locales/de_DE.yaml"), "").unwrap();
        let mut mail = app("mail", &[]);
        mail.categories = vec!["Office".into()];
        let mut chat = app("chat", &[]);
        chat.categories = vec!["Office".into()];
        let shared = serde_yaml::from_str("{ role: { type: string, allowedValues: [developer, design] } }").unwrap();
        let items = check_locales(&dir, &[mail, chat], &[], &shared);
        let _ = fs::remove_dir_all(&dir);
        let item = |name: &str| items.iter().find(|i| i.name == name).unwrap_or_else(|| panic!("no {name}"));
        let messages = |name: &str| item(name).findings.iter().map(|f| (f.severity, f.message.clone())).collect::<Vec<_>>();

        let de = messages("locales/de.yaml");
        assert!(de.iter().all(|(s, _)| *s == Severity::Warning), "{de:?}");
        for unknown in ["app 'nope'", "category 'Ghost'", "allowed value 'chef'"] {
            assert!(de.iter().any(|(_, m)| m.contains(unknown)), "{unknown} in {de:?}");
        }
        assert_eq!(de.len(), 3, "{de:?}");
        assert_eq!(item("locales/de.yaml").coverage.as_deref(), Some("1 of 2 apps, 0 of 0 kits, 1 of 1 category, 1 of 1 option"));
        assert!(messages("locales/fr.yaml").iter().any(|(s, _)| *s == Severity::Error));
        assert!(item("locales/fr.yaml").coverage.is_none());
        assert!(messages("locales/en.yaml").iter().any(|(s, m)| *s == Severity::Warning && m.contains("own texts")));
        assert!(messages("locales/de_DE.yaml").iter().any(|(s, m)| *s == Severity::Error && m.contains("language tag")));
    }

    #[test]
    fn cycles_are_found_and_deep_diamonds_stay_fast() {
        let apps = [app("a", &["b".into()]), app("b", &["c".into()]), app("c", &["a".into()]), app("d", &["a".into()])];
        assert_eq!(cycle(&apps[0], &apps), Some(vec!["a".into(), "b".into(), "c".into(), "a".into()]));
        assert_eq!(cycle(&apps[3], &apps), None, "d leads into a cycle but isn't part of one");
        assert_eq!(cycle(&app("s", &["s".into()]), &[app("s", &["s".into()])]), Some(vec!["s".into(), "s".into()]));

        // 40 layers of two apps, each depending on both apps of the next layer: 2^40 paths.
        let layers = 40;
        let name = |layer: usize, i: usize| format!("l{layer}-{i}");
        let mut apps: Vec<App> = (0..layers)
            .flat_map(|l| {
                let next: Vec<String> = if l + 1 < layers { vec![name(l + 1, 0), name(l + 1, 1)] } else { vec![] };
                [app(&name(l, 0), &next), app(&name(l, 1), &next)]
            })
            .collect();
        let start = std::time::Instant::now();
        for a in &apps {
            assert_eq!(cycle(a, &apps), None);
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(1), "took {:?}", start.elapsed());
        // The bottom layer depending on the top closes a cycle through every layer.
        apps.last_mut().unwrap().dependencies.push(crate::catalog::Item {
            app: name(0, 0),
            required: true,
            selected: true,
            condition: None,
        });
        let found = cycle(&apps[0], &apps).unwrap();
        assert_eq!((found.first(), found.last(), found.len()), (Some(&name(0, 0)), Some(&name(0, 0)), layers + 1));
    }
}
