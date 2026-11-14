//! Options: DSC parameters that change what an app's DSC code does, e.g. `userInstall`.
//!
//! - A collection declares shared options in `collection.yaml` (type, default, label,
//!   description); an app uses an option by declaring it in its DSC `parameters` section, and
//!   may declare app-specific ones there too. Only options the app declares are passed to it.
//! - Values come from, in order: the admin policy for the app (locked), the policy for the
//!   collection (locked), the user's value for the app, the policy's default for the app, the
//!   user's value for the collection, the policy's default for the collection, the collection's
//!   default, the app's own `defaultValue`.
//! - furnish remembers the values an app was installed with (`installed.json`): status checks,
//!   updates and uninstall use those, so changing an option later doesn't orphan an install.
//! - Dependency items may carry a `condition`: a DSC expression over the same values.

use std::{collections::BTreeMap, fs, path::Path};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::catalog::data_dir;

/// An option as declared by a collection or an app.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Declaration {
    /// `bool`, `string` or `int`.
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_values: Vec<Value>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct CollectionFile {
    #[serde(default)]
    parameters: BTreeMap<String, Declaration>,
}

/// Options a collection declares in `collection.yaml` (none if the file is missing).
pub fn collection_declarations(dir: &Path) -> Result<BTreeMap<String, Declaration>> {
    let path = dir.join("collection.yaml");
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let file: CollectionFile = serde_yaml::from_str(&fs::read_to_string(&path)?).with_context(|| format!("{}", path.display()))?;
    Ok(file.parameters)
}

/// Options an app's DSC document declares (`parameters` section).
pub fn app_declarations(doc: &Value) -> BTreeMap<String, Declaration> {
    let Some(params) = doc["parameters"].as_object() else { return BTreeMap::new() };
    params
        .iter()
        .map(|(name, p)| {
            let decl = Declaration {
                kind: p["type"].as_str().unwrap_or("string").to_string(),
                default: p.get("defaultValue").cloned(),
                label: None,
                description: p["description"].as_str().map(String::from),
                allowed_values: p["allowedValues"].as_array().cloned().unwrap_or_default(),
            };
            (name.clone(), decl)
        })
        .collect()
}

/// Where a value comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    Policy,
    App,
    Collection,
    Default,
}

/// An option's value for an app or a collection, with what the UI needs to show and edit it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Resolved {
    pub name: String,
    #[serde(flatten)]
    pub declaration: Declaration,
    pub value: Option<Value>,
    pub source: Source,
    /// The admin policy fixes the value.
    pub locked: bool,
    /// The user's own value at this level, if any.
    pub own_value: Option<Value>,
}

/// Inputs to resolution: user values and policy entries keyed by "collection" or "collection/app".
struct Layers {
    user: BTreeMap<String, BTreeMap<String, Value>>,
    policy: BTreeMap<String, BTreeMap<String, crate::policy::Entry>>,
}

fn layers() -> Layers {
    Layers { user: crate::settings::load().parameters, policy: crate::policy::load().parameters }
}

impl Layers {
    fn user(&self, target: &str, name: &str) -> Option<Value> {
        self.user.get(target).and_then(|m| m.get(name)).cloned()
    }
    fn policy(&self, target: &str, name: &str, locked: bool) -> Option<Value> {
        self.policy.get(target).and_then(|m| m.get(name)).filter(|e| e.locked == locked).map(|e| e.value.clone())
    }
}

/// An app's options (those its DSC document declares), resolved.
pub fn resolve_app(collection: &str, app: &str, doc: &Value, shared: &BTreeMap<String, Declaration>) -> Vec<Resolved> {
    resolve_app_with(&layers(), collection, app, doc, shared)
}

fn resolve_app_with(l: &Layers, collection: &str, app: &str, doc: &Value, shared: &BTreeMap<String, Declaration>) -> Vec<Resolved> {
    let app_key = format!("{collection}/{app}");
    app_declarations(doc)
        .into_iter()
        .map(|(name, own)| {
            // The collection's declaration (label, description, default) applies when it has one.
            let shared_decl = shared.get(&name);
            let declaration = Declaration {
                label: shared_decl.and_then(|d| d.label.clone()).or(own.label.clone()),
                description: own.description.clone().or(shared_decl.and_then(|d| d.description.clone())),
                ..own.clone()
            };
            let candidates = [
                (l.policy(&app_key, &name, true), Source::Policy, true),
                (l.policy(collection, &name, true), Source::Policy, true),
                (l.user(&app_key, &name), Source::App, false),
                (l.policy(&app_key, &name, false), Source::Policy, false),
                (l.user(collection, &name), Source::Collection, false),
                (l.policy(collection, &name, false), Source::Policy, false),
                (shared_decl.and_then(|d| d.default.clone()), Source::Default, false),
                (own.default.clone(), Source::Default, false),
            ];
            let (value, source, locked) = candidates
                .into_iter()
                .find(|(v, _, _)| v.is_some())
                .unwrap_or((None, Source::Default, false));
            Resolved { name: name.clone(), declaration, value, source, locked, own_value: l.user(&app_key, &name) }
        })
        .collect()
}

/// A collection's own options (from `collection.yaml`), resolved at collection level.
pub fn resolve_collection(collection: &str, shared: &BTreeMap<String, Declaration>) -> Vec<Resolved> {
    let l = layers();
    shared
        .iter()
        .map(|(name, decl)| {
            let candidates = [
                (l.policy(collection, name, true), Source::Policy, true),
                (l.user(collection, name), Source::Collection, false),
                (l.policy(collection, name, false), Source::Policy, false),
                (decl.default.clone(), Source::Default, false),
            ];
            let (value, source, locked) =
                candidates.into_iter().find(|(v, _, _)| v.is_some()).unwrap_or((None, Source::Default, false));
            Resolved { name: name.clone(), declaration: decl.clone(), value, source, locked, own_value: l.user(collection, name) }
        })
        .collect()
}

/// name → value, for running DSC.
pub fn values(resolved: &[Resolved]) -> Map<String, Value> {
    resolved.iter().filter_map(|r| r.value.clone().map(|v| (r.name.clone(), v))).collect()
}

/// The document with `values` as the parameters' defaults, so DSC uses them.
pub fn apply(doc: &Value, values: &Map<String, Value>) -> Value {
    let mut doc = doc.clone();
    for (name, value) in values {
        if doc["parameters"].get(name).is_some() {
            doc["parameters"][name]["defaultValue"] = value.clone();
        }
    }
    doc
}

/// Set (or with `None`, clear) the user's value of an option for a target: a collection name,
/// or `collection/app`. Fails if the admin policy locks it or the value doesn't fit.
pub fn set(target: &str, name: &str, value: Option<Value>, declaration: &Declaration) -> Result<()> {
    let policy = crate::policy::load().parameters;
    let collection = target.split('/').next().unwrap_or(target);
    for key in [target, collection] {
        if policy.get(key).and_then(|m| m.get(name)).is_some_and(|e| e.locked) {
            bail!("option '{name}' is set by your organization");
        }
    }
    if let Some(v) = &value {
        check(name, v, declaration)?;
    }
    let mut all = crate::settings::load().parameters;
    let entry = all.entry(target.to_string()).or_default();
    match value {
        Some(v) => {
            entry.insert(name.to_string(), v);
        }
        None => {
            entry.remove(name);
        }
    }
    all.retain(|_, m| !m.is_empty());
    crate::settings::set("parameters", serde_json::to_value(all)?)
}

fn check(name: &str, value: &Value, d: &Declaration) -> Result<()> {
    let fits = match d.kind.as_str() {
        "bool" => value.is_boolean(),
        "int" => value.is_i64(),
        "string" => value.is_string(),
        other => bail!("option '{name}' has type {other}, which furnish can't set"),
    };
    if !fits {
        bail!("option '{name}' needs a {} value", d.kind);
    }
    if !d.allowed_values.is_empty() && !d.allowed_values.contains(value) {
        bail!("option '{name}' must be one of {}", serde_json::to_string(&d.allowed_values)?);
    }
    Ok(())
}

/// Parse a command-line value for an option.
pub fn parse(text: &str, d: &Declaration) -> Result<Value> {
    Ok(match d.kind.as_str() {
        "bool" => match text {
            "true" | "on" | "yes" => Value::Bool(true),
            "false" | "off" | "no" => Value::Bool(false),
            _ => bail!("expected true or false"),
        },
        "int" => Value::from(text.parse::<i64>().context("expected a number")?),
        _ => Value::from(text),
    })
}

// --- Values an app was installed with

fn records_path() -> std::path::PathBuf {
    data_dir().join("installed.json")
}

fn records() -> BTreeMap<String, Map<String, Value>> {
    fs::read_to_string(records_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// The option values an app (`collection/app`) was installed with, if furnish installed it.
pub fn installed_with(app_key: &str) -> Option<Map<String, Value>> {
    records().remove(app_key)
}

pub fn remember(app_key: &str, values: &Map<String, Value>) -> Result<()> {
    let mut all = records();
    all.insert(app_key.to_string(), values.clone());
    write_records(&all)
}

pub fn forget(app_key: &str) -> Result<()> {
    let mut all = records();
    if all.remove(app_key).is_some() {
        write_records(&all)?;
    }
    Ok(())
}

fn write_records(all: &BTreeMap<String, Map<String, Value>>) -> Result<()> {
    fs::create_dir_all(data_dir())?;
    fs::write(records_path(), serde_json::to_string_pretty(all)? + "\n")?;
    Ok(())
}

/// Whether a dependency's `condition` (a DSC expression) holds for these option values.
pub fn condition_holds(condition: &str, values: &Map<String, Value>) -> Result<bool> {
    use dsc_lib::configure::{config_doc::DataType, context::Context};
    let mut context = Context::new();
    for (name, value) in values {
        let kind = match value {
            Value::Bool(_) => DataType::Bool,
            Value::Number(_) => DataType::Int,
            Value::Array(_) => DataType::Array,
            Value::Object(_) => DataType::Object,
            _ => DataType::String,
        };
        context.parameters.insert(name.clone(), (value.clone(), kind));
    }
    let result = dsc_lib::parser::Statement::new()?.parse_and_execute(condition, &context)?;
    Ok(result == Value::Bool(true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(value: Value, locked: bool) -> crate::policy::Entry {
        crate::policy::Entry { value, locked }
    }

    #[test]
    fn app_value_beats_collection_value_beats_default_and_locks_beat_all() {
        let doc = json!({"parameters": {"userInstall": {"type": "bool", "defaultValue": false}}});
        let shared = BTreeMap::from([(
            "userInstall".to_string(),
            Declaration { kind: "bool".into(), default: Some(json!(false)), label: Some("Just for me".into()), description: None, allowed_values: vec![] },
        )]);
        let resolve = |user: Value, policy: Value| {
            let l = Layers {
                user: serde_json::from_value(user).unwrap(),
                policy: serde_json::from_value::<BTreeMap<String, BTreeMap<String, Value>>>(policy)
                    .unwrap()
                    .into_iter()
                    .map(|(k, m)| (k, m.into_iter().map(|(n, v)| (n, entry(v["value"].clone(), v["locked"] == true))).collect()))
                    .collect(),
            };
            let r = resolve_app_with(&l, "c", "kubectl", &doc, &shared).remove(0);
            (r.value.unwrap(), r.source, r.locked)
        };
        assert_eq!(resolve(json!({}), json!({})), (json!(false), Source::Default, false));
        assert_eq!(resolve(json!({"c": {"userInstall": true}}), json!({})), (json!(true), Source::Collection, false));
        assert_eq!(
            resolve(json!({"c": {"userInstall": true}, "c/kubectl": {"userInstall": false}}), json!({})),
            (json!(false), Source::App, false)
        );
        // A locked collection value beats the user's app value.
        assert_eq!(
            resolve(json!({"c/kubectl": {"userInstall": false}}), json!({"c": {"userInstall": {"value": true, "locked": true}}})),
            (json!(true), Source::Policy, true)
        );
        // A policy default yields to the user.
        assert_eq!(
            resolve(json!({"c": {"userInstall": false}}), json!({"c": {"userInstall": {"value": true}}})),
            (json!(false), Source::Collection, false)
        );
    }

    #[test]
    fn dependency_conditions_use_dsc_expressions() {
        let on = Map::from_iter([("userInstall".to_string(), json!(true))]);
        assert!(condition_holds("[parameters('userInstall')]", &on).unwrap());
        assert!(!condition_holds("[not(parameters('userInstall'))]", &on).unwrap());
    }

    #[test]
    fn apply_sets_defaults_of_declared_parameters_only() {
        let doc = json!({"parameters": {"userInstall": {"type": "bool", "defaultValue": false}}});
        let values = Map::from_iter([("userInstall".to_string(), json!(true)), ("other".to_string(), json!(1))]);
        let out = apply(&doc, &values);
        assert_eq!(out["parameters"]["userInstall"]["defaultValue"], json!(true));
        assert!(out["parameters"].get("other").is_none());
    }
}
