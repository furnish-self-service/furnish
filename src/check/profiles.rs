//! Offline target matrix for collection authors. Each app gets its own DSC parameter context.
use super::{Graph, Item, SYSTEMS, error};
use crate::{
    catalog::{App, PACKAGE_TYPE},
    conditions::{TargetContext, host_facts},
    nixos_apps::NixSetup,
    options::Declaration,
    platform::{self, Distribution},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::Path,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Matrix {
    profiles: BTreeMap<String, Profile>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Profile {
    family: String,
    #[serde(default)]
    distribution: String,
    #[serde(default)]
    distribution_like: Vec<String>,
    package_managers: Vec<String>,
    #[serde(default)]
    nix_setup: NixSetup,
    documents_dir: Option<String>,
    #[serde(default)]
    options: Map<String, Value>,
    #[serde(default)]
    app_options: BTreeMap<String, Map<String, Value>>,
    #[serde(default)]
    expect: BTreeMap<String, Expected>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Expected {
    steps: Option<Vec<String>>,
    skipped: Option<Vec<String>>,
    dependencies: Option<Vec<String>>,
    packages: Option<BTreeMap<String, Package>>,
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Package {
    manager: Option<String>,
    name: String,
}

#[derive(Default)]
struct Decisions {
    steps: Vec<String>,
    skipped: Vec<String>,
    dependencies: Vec<String>,
    packages: BTreeMap<String, Package>,
}

pub(super) fn check(path: &Path, dir: &Path, apps: &[App], shared: &BTreeMap<String, Declaration>) -> Result<Vec<Item>> {
    let matrix: Matrix = serde_yaml::from_slice(&fs::read(path).with_context(|| format!("reading {}", path.display()))?)
        .with_context(|| format!("invalid profiles file {}", path.display()))?;
    ensure!(!matrix.profiles.is_empty(), "no target profiles in {}", path.display());
    let scripts = crate::options::package_managers(dir)?;
    let mut results = Vec::new();
    for (name, profile) in &matrix.profiles {
        let mut item =
            Item { name: name.clone(), kind: "profile", findings: Vec::new(), details: Vec::new(), systems: Vec::new(), coverage: None };
        if let Err(err) = profile.check(apps, shared, &scripts, &mut item) {
            item.findings.push(error(format!("{err:#}")));
        }
        results.push(item);
    }
    Ok(results)
}

impl Profile {
    fn check(
        &self,
        apps: &[App],
        shared: &BTreeMap<String, Declaration>,
        scripts: &BTreeMap<String, crate::script_managers::Definition>,
        item: &mut Item,
    ) -> Result<()> {
        let family = SYSTEMS.into_iter().find(|&s| s == self.family).context("family must be Windows, macOS or Linux")?;
        ensure!(
            family == "Linux" || (self.distribution.is_empty() && self.distribution_like.is_empty()),
            "only Linux profiles have distribution or distributionLike"
        );
        for id in std::iter::once(&self.distribution).filter(|id| !id.is_empty()).chain(&self.distribution_like) {
            ensure!(platform::valid_distribution_id(id), "invalid distribution identifier '{id}'");
        }
        let mut seen = HashSet::new();
        let (mut native, mut scripted) = (Vec::new(), Vec::new());
        for id in &self.package_managers {
            ensure!(seen.insert(id), "duplicate package manager '{id}'");
            if platform::supported_manager(id) {
                ensure!(platform::manager_on_platform(id, family == "Windows"), "'{id}' is unavailable on {family}");
                native.push(id.as_str());
            } else {
                let script = scripts.get(id).with_context(|| format!("package manager '{id}' is not declared by this collection"))?;
                ensure!(
                    script.supports_platform(family == "Windows"),
                    "package manager '{id}' needs an interpreter for {} on {family}",
                    script.script
                );
                scripted.push(id.as_str());
            }
        }
        ensure!(!self.nix_setup.system || family == "Linux", "nixSetup.system requires a Linux profile");
        ensure!(!self.nix_setup.home || family != "Windows", "nixSetup.home requires a Linux or macOS profile");
        ensure!(!(self.nix_setup.system || self.nix_setup.home) || native.contains(&"nix"),
            "nixSetup requires nix in packageManagers");
        for key in self.expect.keys().chain(self.app_options.keys()) {
            ensure!(apps.iter().any(|app| &app.id == key), "unknown app '{key}'");
        }
        let mut values: Map<_, _> = shared.iter().filter_map(|(key, d)| Some((key.clone(), d.default.clone()?))).collect();
        for (key, value) in &self.options {
            crate::options::check(key, value, shared.get(key).with_context(|| format!("unknown collection option '{key}'"))?)?;
            values.insert(key.clone(), value.clone());
        }
        let distro = Distribution { id: self.distribution.clone(), like: self.distribution_like.clone() };
        let mut ordered_native = native.clone();
        ordered_native.sort_by_key(|id| platform::native_manager_priority(id));
        ensure!(native == ordered_native, "native package managers must follow Furnish priority: {}", ordered_native.join(", "));
        scripted.sort();
        let managers: Vec<_> = native.iter().chain(&scripted).collect();
        let host = host_facts(&distro, &managers, self.nix_setup, self.documents_dir.as_deref());
        let target = Target { family, host, distro, native, scripted, nix_setup: self.nix_setup };
        let mut graph = Graph::default();
        for app in apps {
            let checked = (|| {
                let mut input: Map<_, _> =
                    values.iter().filter(|(k, _)| app.doc["parameters"].get(k).is_some()).map(|(k, v)| (k.clone(), v.clone())).collect();
                if let Some(overrides) = self.app_options.get(&app.id) {
                    for (key, value) in overrides {
                        ensure!(app.doc["parameters"].get(key).is_some(), "unknown app option '{key}'");
                        if let Some(decl) = shared.get(key)
                            && app.doc["parameters"][key].get("allowedValues").is_none()
                        {
                            crate::options::check(key, value, decl)?;
                        }
                        input.insert(key.clone(), value.clone());
                    }
                }
                let mut ctx = TargetContext::new(family, target.host.clone(), &app.doc, input)?;
                let actual = target.decisions(app, &mut ctx, &mut item.details)?;
                for dep in &actual.dependencies {
                    graph.edge(&app.id, dep, Some(true));
                }
                if let Some(expected) = self.expect.get(&app.id) {
                    expected.compare(&actual, &app.id, item);
                }
                Ok::<_, anyhow::Error>(())
            })();
            if let Err(err) = checked {
                item.findings.push(error(format!("{}: {err:#}", app.id)));
            }
        }
        for app in apps {
            if let Some(finding) = graph.finding(&app.id, &item.name, false) {
                item.findings.push(finding);
            }
        }
        Ok(())
    }
}

struct Target<'a> {
    family: &'static str,
    host: Value,
    distro: Distribution,
    native: Vec<&'a str>,
    scripted: Vec<&'a str>,
    nix_setup: NixSetup,
}

impl Target<'_> {
    fn decisions(&self, app: &App, ctx: &mut TargetContext, details: &mut Vec<String>) -> Result<Decisions> {
        let mut result = Decisions::default();
        for dep in &app.dependencies {
            if let Some(expr) = &dep.condition
                && !ctx.condition(expr).with_context(|| format!("dependency {}: {expr}", dep.app))?
            {
                details.push(format!("{}: skipped dependency {} ({expr} is false)", app.id, dep.app));
                continue;
            }
            result.dependencies.push(dep.app.clone());
        }
        let deps = if result.dependencies.is_empty() { "none".into() } else { result.dependencies.join(", ") };
        details.push(format!("{}: active dependencies: {deps}", app.id));
        let resources = app.doc["resources"].as_array().context("resources must be a list")?;
        for i in crate::engine::invocation_order(resources)? {
            let r = &resources[i];
            let name = r["name"].as_str().context("step needs a name")?;
            let setup = crate::nixos_apps::only_if_set_up(r);
            if setup && self.family == "Windows" {
                details.push(format!("{}: skipped {name} (NixOS and Home Manager configurations are unavailable on Windows)", app.id));
                result.skipped.push(name.into());
                continue;
            }
            if let Some(expr) = r["condition"].as_str()
                && !ctx.condition(expr).with_context(|| format!("{name}: {expr}"))?
            {
                details.push(format!("{}: skipped {name} ({expr} is false)", app.id));
                result.skipped.push(name.into());
                continue;
            }
            if setup {
                let reason = if !self.nix_setup.system && !self.nix_setup.home {
                    Some("no NixOS or Home Manager configuration is set up in this profile")
                } else {
                    let props = ctx.properties(&r["properties"]).with_context(|| format!("{name}: NixOS properties"))?;
                    crate::nixos_apps::missing_profile_destination(&props, self.nix_setup.system, self.nix_setup.home)
                        .with_context(|| format!("{name}: NixOS properties"))?
                };
                if let Some(reason) = reason {
                    details.push(format!("{}: skipped {name} ({reason})", app.id));
                    result.skipped.push(name.into());
                    continue;
                }
            }
            result.steps.push(name.into());
            if r["type"].as_str().is_some_and(|t| t.eq_ignore_ascii_case(PACKAGE_TYPE)) {
                let props = ctx.properties(&r["properties"]).with_context(|| format!("{name}: package properties"))?;
                crate::package_names::validate(&props).with_context(|| name.to_owned())?;
                let selected = crate::package_names::select(&props, &self.distro, &self.native, &self.scripted, self.family == "Windows")
                    .with_context(|| name.to_owned())?;
                let availability = match selected.manager.as_deref() {
                    Some(id) if !self.native.contains(&id) && !self.scripted.contains(&id) => {
                        format!("missing prerequisite (package manager {id}) for")
                    }
                    None => "missing prerequisite (a package manager) for".into(),
                    _ => "run".into(),
                };
                details.push(format!("{}: {availability} {name}: {}", app.id, selected.explanation()));
                result.packages.insert(name.into(), Package { name: selected.name.into(), manager: selected.manager });
            } else {
                details.push(format!("{}: run {name}", app.id));
            }
        }
        Ok(result)
    }
}

impl Expected {
    fn compare(&self, result: &Decisions, app: &str, item: &mut Item) {
        fn compare<T: PartialEq + Serialize>(expected: &Option<T>, actual: &T, field: &str, app: &str, item: &mut Item) {
            if let Some(expected) = expected
                && expected != actual
            {
                item.findings.push(error(format!("{app}: expected {field} {}, got {}", json!(expected), json!(actual))));
            }
        }
        compare(&self.steps, &result.steps, "steps", app, item);
        compare(&self.skipped, &result.skipped, "skipped", app, item);
        compare(&self.dependencies, &result.dependencies, "dependencies", app, item);
        compare(&self.packages, &result.packages, "packages", app, item);
    }
}
