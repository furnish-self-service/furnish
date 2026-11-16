//! Translations of a collection's texts: `locales/<lang>.yaml`, one file per language (a BCP 47
//! tag such as `de` or `pt-BR`). The collection's own texts are in `language` (collection.yaml,
//! default `en`).
//!
//! Which text is shown: the one in the app's language if the collection has it, else the one in
//! the fallback language (setting `fallbackLanguage`), else the collection's own. A language
//! matches by its exact tag, then by its primary subtag (`pt-BR` finds `pt`, then `pt-PT`).
//! The desktop app does the same in gui/src/lib/content.ts.

use std::{collections::BTreeMap, fs, path::Path};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// One language's translations.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocaleFile {
    /// Category key (as the apps write it) → shown name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub categories: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub apps: BTreeMap<String, Texts>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub kits: BTreeMap<String, Texts>,
    /// Options of the collection (collection.yaml) and of its apps, by name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub parameters: BTreeMap<String, ParameterTexts>,
}

/// An app's or a kit's texts.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Texts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// An option's texts.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParameterTexts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Allowed value (`true`, `1` and `developer` all as text) → shown label.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty", deserialize_with = "scalar_keys")]
    pub allowed_values: BTreeMap<String, String>,
}

/// Map keys of any scalar type as text: YAML reads `true:` and `1:` as a bool and a number.
fn scalar_keys<'de, D: serde::Deserializer<'de>>(d: D) -> Result<BTreeMap<String, String>, D::Error> {
    use serde::de::Error;
    let map = serde_yaml::Mapping::deserialize(d)?;
    map.into_iter()
        .map(|(k, v)| {
            let key = match k {
                serde_yaml::Value::String(s) => s,
                serde_yaml::Value::Bool(b) => b.to_string(),
                serde_yaml::Value::Number(n) => n.to_string(),
                _ => return Err(D::Error::custom("allowedValues: keys must be values such as developer, true or 1")),
            };
            let serde_yaml::Value::String(label) = v else {
                return Err(D::Error::custom(format!("allowedValues: the label for '{key}' must be text")));
            };
            Ok((key, label))
        })
        .collect()
}

/// The text form of an allowed value, as `allowedValues` keys write it.
pub fn value_key(value: &serde_json::Value) -> String {
    value.as_str().map(String::from).unwrap_or_else(|| value.to_string())
}

/// A collection's translations and the language of its own texts.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Locales {
    /// The language the collection's own texts are written in.
    pub language: String,
    pub files: BTreeMap<String, LocaleFile>,
}

/// Whether `tag` looks like a BCP 47 language tag: `de`, `pt-BR`, `zh-Hant-TW`.
pub fn valid_tag(tag: &str) -> bool {
    let mut parts = tag.split('-');
    let primary = parts.next().unwrap_or_default();
    (2..=3).contains(&primary.len())
        && primary.chars().all(|c| c.is_ascii_alphabetic())
        && parts.all(|p| (1..=8).contains(&p.len()) && p.chars().all(|c| c.is_ascii_alphanumeric()))
}

fn primary(tag: &str) -> String {
    tag.split('-').next().unwrap_or_default().to_ascii_lowercase()
}

/// The best of `available` for `wanted`: the same tag, else the primary subtag alone, else the
/// first (sorted) with the same primary subtag. Ignores case.
pub fn best<'a>(wanted: &str, available: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let mut available: Vec<&str> = available.into_iter().collect();
    available.sort();
    let wanted_primary = primary(wanted);
    available
        .iter()
        .find(|a| a.eq_ignore_ascii_case(wanted))
        .or_else(|| available.iter().find(|a| a.eq_ignore_ascii_case(&wanted_primary)))
        .or_else(|| available.iter().find(|a| primary(a) == wanted_primary))
        .copied()
}

/// The `locales/` files of a collection and `language` from its collection.yaml. Files that
/// don't load are left out, each with its error.
pub fn load(dir: &Path) -> (Locales, Vec<(String, anyhow::Error)>) {
    let language = crate::options::collection_language(dir);
    let mut files = BTreeMap::new();
    let mut errors = Vec::new();
    for (tag, path) in locale_paths(dir) {
        match load_file(&path) {
            Ok(file) => {
                files.insert(tag, file);
            }
            Err(err) => errors.push((format!("locales/{tag}.yaml"), err)),
        }
    }
    (Locales { language, files }, errors)
}

/// `locales/*.yaml` by language tag (the file name), sorted; names that aren't tags too.
pub fn locale_paths(dir: &Path) -> Vec<(String, std::path::PathBuf)> {
    let mut paths: Vec<_> = fs::read_dir(dir.join("locales"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "yaml"))
        .filter_map(|p| Some((p.file_stem()?.to_str()?.to_string(), p)))
        .collect();
    paths.sort();
    paths
}

pub fn load_file(path: &Path) -> Result<LocaleFile> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    if text.trim().is_empty() {
        return Ok(LocaleFile::default());
    }
    Ok(serde_yaml::from_str(&text)?)
}

/// Warnings for a collection's locale files (printed once per process, like other load warnings).
pub fn load_reporting(dir: &Path) -> Locales {
    let (locales, errors) = load(dir);
    for (file, err) in errors {
        eprintln!("warning: ignoring {}: {err:#}", dir.join(file).display());
    }
    locales
}

impl Locales {
    /// The translation to show for one text, in the order of `wanted` (the app's language, then
    /// the fallback language); `None`: the collection's own text. A wanted language the
    /// collection's own texts are in ends the search with those.
    pub fn text<'a>(&'a self, wanted: &[String], get: impl Fn(&'a LocaleFile) -> Option<&'a String>) -> Option<&'a str> {
        let available = || self.files.keys().map(String::as_str).chain([self.language.as_str()]);
        for w in wanted {
            match best(w, available()) {
                Some(tag) if tag.eq_ignore_ascii_case(&self.language) => return None,
                Some(tag) => {
                    if let Some(text) = self.files.get(tag).and_then(&get) {
                        return Some(text);
                    }
                }
                None => {}
            }
        }
        None
    }

    /// The name to show: translated, else the app's own (`metadata.furnish.name`), else its id.
    pub fn app_name(&self, wanted: &[String], id: &str, own: Option<&str>) -> String {
        self.text(wanted, |f| f.apps.get(id)?.name.as_ref()).or(own).unwrap_or(id).to_string()
    }

    pub fn kit_name(&self, wanted: &[String], id: &str, own: Option<&str>) -> String {
        self.text(wanted, |f| f.kits.get(id)?.name.as_ref()).or(own).unwrap_or(id).to_string()
    }

    pub fn app_description<'a>(&'a self, wanted: &[String], id: &str, own: &'a str) -> &'a str {
        self.text(wanted, |f| f.apps.get(id)?.description.as_ref()).unwrap_or(own)
    }

    pub fn kit_description<'a>(&'a self, wanted: &[String], id: &str, own: &'a str) -> &'a str {
        self.text(wanted, |f| f.kits.get(id)?.description.as_ref()).unwrap_or(own)
    }

    pub fn category<'a>(&'a self, wanted: &[String], key: &'a str) -> &'a str {
        self.text(wanted, |f| f.categories.get(key)).unwrap_or(key)
    }

    /// An option's label, description and choice labels in place, as the CLI shows them.
    pub fn translate_option(&self, wanted: &[String], r: &mut crate::options::Resolved) {
        let name = r.name.clone();
        if let Some(label) = self.text(wanted, |f| f.parameters.get(&name)?.label.as_ref()) {
            r.declaration.label = Some(label.to_string());
        }
        if let Some(description) = self.text(wanted, |f| f.parameters.get(&name)?.description.as_ref()) {
            r.declaration.description = Some(description.to_string());
        }
        for choice in &mut r.choices {
            let key = value_key(&choice.value);
            if let Some(label) = self.text(wanted, |f| f.parameters.get(&name)?.allowed_values.get(&key)) {
                choice.label = Some(label.to_string());
            }
        }
    }
}

/// The languages to show collection texts in, in order: the app's language (setting `language`;
/// `system` follows the system language if the app has it, like the desktop app), then the
/// fallback language.
pub fn wanted() -> Vec<String> {
    let settings = crate::settings::load();
    vec![ui_language(settings.language, system_language().as_deref()), settings.fallback_language]
}

/// The tag of the app's language. For `system`, the system's own tag (e.g. `pt-BR`) if the app
/// has its primary language, else English.
pub fn ui_language(setting: crate::settings::Language, system: Option<&str>) -> String {
    if setting != crate::settings::Language::System {
        return serde_json::to_value(setting).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_else(|| "en".into());
    }
    let system = system.map(|s| s.replace('_', "-")).filter(|s| valid_tag(s));
    match system {
        Some(tag) if crate::settings::UI_LANGUAGES.contains(&primary(&tag).as_str()) => tag,
        _ => "en".into(),
    }
}

/// The system language for the terminal: LC_ALL, LC_MESSAGES or LANG (e.g. `de_DE.UTF-8`).
fn system_language() -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|var| {
        let value = std::env::var(var).ok()?;
        let tag = value.split(['.', '@']).next()?.to_string();
        (!tag.is_empty() && tag != "C" && tag != "POSIX").then_some(tag)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Language;

    fn locales(language: &str, files: &[(&str, &str)]) -> Locales {
        Locales {
            language: language.into(),
            files: files.iter().map(|(tag, yaml)| (tag.to_string(), serde_yaml::from_str(yaml).unwrap())).collect(),
        }
    }

    fn w(tags: &[&str]) -> Vec<String> {
        tags.iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn matching_takes_the_exact_tag_then_the_primary_subtag() {
        let available = ["de", "pt", "pt-BR", "zh-Hant"];
        assert_eq!(best("pt-BR", available), Some("pt-BR"));
        assert_eq!(best("pt-PT", available), Some("pt"));
        assert_eq!(best("PT-br", available), Some("pt-BR"));
        assert_eq!(best("de-AT", available), Some("de"));
        assert_eq!(best("zh", available), Some("zh-Hant"));
        assert_eq!(best("pt", ["pt-PT", "pt-BR"]), Some("pt-BR"), "first by sort order");
        assert_eq!(best("fr", available), None);
    }

    #[test]
    fn the_apps_language_then_the_fallback_then_the_collections_own_texts() {
        let l = locales(
            "en",
            &[
                ("de", "apps: { mail: { description: E-Mail } }\ncategories: { Office: Büro }"),
                ("fr", "apps: { mail: { description: Courriel }, other: { description: Autre } }"),
            ],
        );
        let desc = |wanted: &[&str], id| l.app_description(&w(wanted), id, "own").to_string();
        assert_eq!(desc(&["de", "en"], "mail"), "E-Mail");
        assert_eq!(desc(&["de-AT", "en"], "mail"), "E-Mail");
        // Not translated into German: the fallback language, here French.
        assert_eq!(desc(&["de", "fr"], "other"), "Autre");
        // The fallback is the collection's own language: its own text.
        assert_eq!(desc(&["de", "en"], "other"), "own");
        assert_eq!(desc(&["it", "en"], "mail"), "own");
        assert_eq!(desc(&["it", "pl"], "mail"), "own");
        // English is the collection's language: its own texts, even though French comes next.
        assert_eq!(desc(&["en", "fr"], "mail"), "own");
        assert_eq!(l.category(&w(&["de", "en"]), "Office"), "Büro");
        assert_eq!(l.category(&w(&["fr", "en"]), "Office"), "Office");
    }

    #[test]
    fn regional_files_serve_the_primary_language_and_back() {
        let l = locales("en", &[("pt", "categories: { Office: Escritório }"), ("de-CH", "categories: { Office: Büro (CH) }")]);
        assert_eq!(l.category(&w(&["pt-BR", "en"]), "Office"), "Escritório");
        assert_eq!(l.category(&w(&["de", "en"]), "Office"), "Büro (CH)");
    }

    #[test]
    fn a_collection_in_another_language_counts_as_that_language() {
        // A German-only collection: its texts for German users, and as the last resort.
        let l = locales("de", &[("en", "apps: { mail: { name: Mail, description: Email } }")]);
        assert_eq!(l.app_description(&w(&["de", "en"]), "mail", "E-Mail"), "E-Mail");
        assert_eq!(l.app_description(&w(&["fr", "en"]), "mail", "E-Mail"), "Email");
        assert_eq!(l.app_description(&w(&["fr", "it"]), "mail", "E-Mail"), "E-Mail");
        assert_eq!(l.app_name(&w(&["fr", "en"]), "mail", None), "Mail");
        // German, also by region: the collection's own texts, not the English file.
        assert_eq!(l.app_name(&w(&["de-AT", "en"]), "mail", Some("E-Mail-Programm")), "E-Mail-Programm");
        assert_eq!(l.app_name(&w(&["de", "en"]), "mail", None), "mail");
    }

    #[test]
    fn options_translate_label_description_and_choices() {
        let l = locales("en", &[("de", "parameters: { role: { label: Rolle, allowedValues: { developer: Entwicklung, true: Ja, 1: Eins } } }")]);
        let decl: crate::options::Declaration = serde_yaml::from_str(
            "{ type: string, label: Role, description: For you, allowedValues: [{ value: developer, label: Dev }, { value: design, label: Design }, true, 1] }",
        )
        .unwrap();
        let mut r = crate::options::resolve_collection("c", &[("role".to_string(), decl)].into()).remove(0);
        l.translate_option(&w(&["de", "en"]), &mut r);
        assert_eq!(r.declaration.label.as_deref(), Some("Rolle"));
        assert_eq!(r.declaration.description.as_deref(), Some("For you"));
        let labels: Vec<_> = r.choices.iter().map(|c| c.label.as_deref()).collect();
        assert_eq!(labels, [Some("Entwicklung"), Some("Design"), Some("Ja"), Some("Eins")]);
    }

    #[test]
    fn the_terminal_follows_the_system_language_like_the_desktop_app() {
        assert_eq!(ui_language(Language::System, Some("pt_BR")), "pt-BR");
        assert_eq!(ui_language(Language::System, Some("de-AT")), "de-AT");
        assert_eq!(ui_language(Language::System, Some("sv_SE")), "en");
        assert_eq!(ui_language(Language::System, None), "en");
        assert_eq!(ui_language(Language::Fr, Some("de_DE")), "fr");
    }

    #[test]
    fn tags() {
        for ok in ["de", "pt-BR", "zh-Hant-TW", "gsw"] {
            assert!(valid_tag(ok), "{ok}");
        }
        for bad in ["", "d", "deutsch", "de_DE", "pt-", "de DE"] {
            assert!(!valid_tag(bad), "{bad}");
        }
    }
}
