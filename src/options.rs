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
    /// The only values allowed: plain values, or `{value, label}` (labels in `collection.yaml`
    /// only; DSC wants plain values in app documents).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_values: Vec<Value>,
    /// Users must choose a value (asked when the collection is added, or on the next launch).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub required: bool,
}

/// One of an option's allowed values, with what to show for it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Choice {
    pub value: Value,
    pub label: Option<String>,
}

impl Declaration {
    pub fn choices(&self) -> Vec<Choice> {
        self.allowed_values
            .iter()
            .map(|entry| match entry.get("value") {
                Some(value) => Choice { value: value.clone(), label: entry["label"].as_str().map(String::from) },
                None => Choice { value: entry.clone(), label: None },
            })
            .collect()
    }
}

/// `collection.yaml`: the collection's avatar and the options its apps share.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct CollectionFile {
    #[serde(default)]
    parameters: BTreeMap<String, Declaration>,
    /// Image file in the collection (SVG, PNG, JPEG or WebP).
    icon: Option<String>,
    /// The image shown instead of `icon` in dark mode.
    #[serde(rename = "iconDark")]
    icon_dark: Option<String>,
    /// The language of the collection's own texts (a BCP 47 tag; default `en`), see `locale`.
    language: Option<String>,
}

fn collection_file(dir: &Path) -> Result<CollectionFile> {
    let path = dir.join("collection.yaml");
    if !path.exists() {
        return Ok(CollectionFile::default());
    }
    serde_yaml::from_str(&fs::read_to_string(&path)?).with_context(|| format!("{}", path.display()))
}

/// Options a collection declares in `collection.yaml` (none if the file is missing).
pub fn collection_declarations(dir: &Path) -> Result<BTreeMap<String, Declaration>> {
    Ok(collection_file(dir)?.parameters)
}

/// The collection's avatar and the one for dark mode (`icon`, `iconDark` in `collection.yaml`),
/// relative to the collection.
#[derive(Default)]
pub struct CollectionIcons {
    pub icon: Option<String>,
    pub icon_dark: Option<String>,
}

pub fn collection_icons(dir: &Path) -> CollectionIcons {
    collection_file(dir).map(|f| CollectionIcons { icon: f.icon, icon_dark: f.icon_dark }).unwrap_or_default()
}

/// The language of the collection's own texts (`language` in `collection.yaml`, default `en`).
pub fn collection_language(dir: &Path) -> String {
    collection_file(dir).ok().and_then(|f| f.language).unwrap_or_else(|| "en".into())
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
                required: false,
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
    /// The allowed values with their labels (a dropdown); empty: any value of the type.
    pub choices: Vec<Choice>,
    /// Required, but nobody chose a value yet.
    pub missing: bool,
}

/// The required options that still need a value.
pub fn missing(resolved: &[Resolved]) -> Vec<&Resolved> {
    resolved.iter().filter(|r| r.missing).collect()
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
            // Allowed values: the app's own, labelled as the collection labels them.
            let shared_choices = shared_decl.map(Declaration::choices).unwrap_or_default();
            let allowed_values = if own.allowed_values.is_empty() {
                shared_decl.map(|d| d.allowed_values.clone()).unwrap_or_default()
            } else {
                own.allowed_values
                    .iter()
                    .map(|v| match shared_choices.iter().find(|c| &c.value == v).and_then(|c| c.label.clone()) {
                        Some(label) => serde_json::json!({ "value": v, "label": label }),
                        None => v.clone(),
                    })
                    .collect()
            };
            let declaration = Declaration {
                label: shared_decl.and_then(|d| d.label.clone()).or(own.label.clone()),
                description: own.description.clone().or(shared_decl.and_then(|d| d.description.clone())),
                required: shared_decl.is_some_and(|d| d.required),
                allowed_values,
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
            let missing = declaration.required && value.is_none();
            Resolved {
                name: name.clone(),
                choices: declaration.choices(),
                declaration,
                value,
                source,
                locked,
                own_value: l.user(&app_key, &name),
                missing,
            }
        })
        .collect()
}

/// A collection's own options (from `collection.yaml`), resolved at collection level.
pub fn resolve_collection(collection: &str, shared: &BTreeMap<String, Declaration>) -> Vec<Resolved> {
    resolve_collection_with(&layers(), collection, shared)
}

fn resolve_collection_with(l: &Layers, collection: &str, shared: &BTreeMap<String, Declaration>) -> Vec<Resolved> {
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
            Resolved {
                name: name.clone(),
                declaration: decl.clone(),
                missing: decl.required && value.is_none(),
                choices: decl.choices(),
                value,
                source,
                locked,
                own_value: l.user(collection, name),
            }
        })
        .collect()
}

/// name → value, for running DSC.
pub fn values(resolved: &[Resolved]) -> Map<String, Value> {
    resolved.iter().filter_map(|r| r.value.clone().map(|v| (r.name.clone(), v))).collect()
}

/// The document with `values` as the parameters' defaults, so DSC uses them. DSC evaluates a
/// string starting with `[` as an expression, so such a value gets another `[` in front (DSC's
/// escape for a literal `[`); the app's own default stays as its author wrote it.
pub fn apply(doc: &Value, values: &Map<String, Value>) -> Value {
    let mut doc = doc.clone();
    for (name, value) in values {
        let Some(param) = doc["parameters"].get_mut(name) else { continue };
        if param.get("defaultValue") == Some(value) {
            continue;
        }
        param["defaultValue"] = match value {
            Value::String(s) if s.starts_with('[') => Value::String(format!("[{s}")),
            other => other.clone(),
        };
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
    let allowed: Vec<Value> = d.choices().into_iter().map(|c| c.value).collect();
    if !allowed.is_empty() && !allowed.contains(value) {
        bail!("option '{name}' must be one of {}", serde_json::to_string(&allowed)?);
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

// Kept in the data directory of whoever installs: the user's, or the furnish service's.

fn records(dir: &Path) -> BTreeMap<String, Map<String, Value>> {
    fs::read_to_string(dir.join("installed.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// The option values an app (`collection/app`) was installed with, if furnish installed it.
pub fn installed_with(dir: &Path, app_key: &str) -> Option<Map<String, Value>> {
    records(dir).remove(app_key)
}

pub fn remember(dir: &Path, app_key: &str, values: &Map<String, Value>) -> Result<()> {
    let mut all = records(dir);
    all.insert(app_key.to_string(), values.clone());
    write_records(dir, &all)
}

pub fn forget(dir: &Path, app_key: &str) -> Result<()> {
    let mut all = records(dir);
    if all.remove(app_key).is_some() {
        write_records(dir, &all)?;
    }
    Ok(())
}

fn write_records(dir: &Path, all: &BTreeMap<String, Map<String, Value>>) -> Result<()> {
    fs::create_dir_all(dir)?;
    fs::write(dir.join("installed.json"), serde_json::to_string_pretty(all)? + "\n")?;
    Ok(())
}

/// Values a client asks the furnish service to install with, checked against the app's options:
/// only declared ones, of the right type, never against a policy lock, and no free text (it would
/// reach resource commands that run as root). Options not asked for keep their resolved value.
pub fn accept(resolved: &[Resolved], requested: &Map<String, Value>) -> Result<Map<String, Value>> {
    let mut accepted = values(resolved);
    for (name, value) in requested {
        let r = resolved.iter().find(|r| &r.name == name).with_context(|| format!("unknown option '{name}'"))?;
        if r.locked {
            if r.value.as_ref() != Some(value) {
                bail!("option '{name}' is set by your organization");
            }
            continue;
        }
        check(name, value, &r.declaration)?;
        if r.declaration.kind == "string" && r.choices.is_empty() {
            bail!("option '{name}' takes free text, which system-wide installs don't accept (declare allowedValues)");
        }
        accepted.insert(name.clone(), value.clone());
    }
    Ok(accepted)
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
    fn labelled_choices_and_required_options() {
        let role: Declaration = serde_yaml::from_str(
            "type: string\nrequired: true\nallowedValues: [{value: dev, label: Software developer}, ops]",
        )
        .unwrap();
        assert_eq!(role.choices(), [
            Choice { value: json!("dev"), label: Some("Software developer".into()) },
            Choice { value: json!("ops"), label: None },
        ]);
        assert!(check("role", &json!("dev"), &role).is_ok());
        assert!(check("role", &json!("Software developer"), &role).is_err(), "labels aren't values");
        let shared = BTreeMap::from([("role".to_string(), role)]);
        let empty = Layers { user: BTreeMap::new(), policy: BTreeMap::new() };
        // Unanswered, the collection's option is missing; once the user answers, it isn't.
        let r = &resolve_collection_with(&empty, "c", &shared)[0];
        assert!(r.missing && r.value.is_none());
        let answered = Layers { user: serde_json::from_value(json!({"c": {"role": "ops"}})).unwrap(), policy: BTreeMap::new() };
        assert!(!resolve_collection_with(&answered, "c", &shared)[0].missing);
        // An app declaring the option with plain values gets the collection's labels.
        let doc = json!({"parameters": {"role": {"type": "string", "allowedValues": ["dev", "ops"]}}});
        let app = &resolve_app_with(&answered, "c", "tool", &doc, &shared)[0];
        assert_eq!(app.choices[0].label.as_deref(), Some("Software developer"));
    }

    #[test]
    fn the_service_accepts_only_declared_typed_unlocked_choices() {
        let option = |name: &str, kind: &str, allowed: Vec<Value>, value: Value, locked: bool| Resolved {
            name: name.into(),
            choices: allowed.iter().map(|v| Choice { value: v.clone(), label: None }).collect(),
            missing: false,
            declaration: Declaration { kind: kind.into(), default: None, label: None, description: None, allowed_values: allowed, required: false },
            value: Some(value),
            source: if locked { Source::Policy } else { Source::Default },
            locked,
            own_value: None,
        };
        let resolved = [
            option("userInstall", "bool", vec![], json!(false), false),
            option("channel", "string", vec![json!("stable"), json!("beta")], json!("stable"), false),
            option("prefix", "string", vec![], json!("/opt"), false),
            option("proxy", "bool", vec![], json!(true), true),
        ];
        let ask = |v: Value| accept(&resolved, v.as_object().unwrap());
        let ok = ask(json!({"userInstall": true, "channel": "beta", "proxy": true})).unwrap();
        assert_eq!(Value::Object(ok), json!({"userInstall": true, "channel": "beta", "prefix": "/opt", "proxy": true}));
        assert!(ask(json!({"other": 1})).is_err(), "undeclared");
        assert!(ask(json!({"userInstall": "yes"})).is_err(), "wrong type");
        assert!(ask(json!({"channel": "nightly"})).is_err(), "not allowed");
        assert!(ask(json!({"prefix": "/tmp; rm -rf /"})).is_err(), "free text");
        assert!(ask(json!({"proxy": false})).is_err(), "locked");
    }

    #[test]
    fn app_value_beats_collection_value_beats_default_and_locks_beat_all() {
        let doc = json!({"parameters": {"userInstall": {"type": "bool", "defaultValue": false}}});
        let shared = BTreeMap::from([(
            "userInstall".to_string(),
            Declaration { kind: "bool".into(), default: Some(json!(false)), label: Some("Just for me".into()), description: None, allowed_values: vec![], required: false },
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

    #[test]
    fn apply_keeps_values_starting_with_a_bracket_literal() {
        let doc = json!({"parameters": {
            "prefix": {"type": "string", "defaultValue": "/opt"},
            "own": {"type": "string", "defaultValue": "[concat('a', 'b')]"},
        }});
        let values = Map::from_iter([
            ("prefix".to_string(), json!("[envvar('HOME')]")),
            ("own".to_string(), json!("[concat('a', 'b')]")),
        ]);
        let out = apply(&doc, &values);
        let prefix = out["parameters"]["prefix"]["defaultValue"].as_str().unwrap();
        assert_eq!(prefix, "[[envvar('HOME')]");
        // DSC reads the escaped string back as the literal value.
        let literal = dsc_lib::parser::Statement::new()
            .unwrap()
            .parse_and_execute(prefix, &dsc_lib::configure::context::Context::new())
            .unwrap();
        assert_eq!(literal, json!("[envvar('HOME')]"));
        // The app's own default, unchanged, is still the author's expression.
        assert_eq!(out["parameters"]["own"]["defaultValue"], json!("[concat('a', 'b')]"));
    }
}
