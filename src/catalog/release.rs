//! Compare two committed trees. Parsing declarations here must never discover or invoke DSC
//! resources, materialize historical app versions, or write files from the candidate tree.
use super::{Collection, data_dir, git_bytes, hold_applies, origin_url};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewedRevisions {
    pub current: String,
    pub candidate: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Change {
    Added,
    Removed,
    Changed,
}

impl Change {
    pub fn label(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Removed => "removed",
            Self::Changed => "changed",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StepChange {
    pub name: String,
    pub kind: String,
    pub removal: bool,
    pub change: Change,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AppChange {
    pub id: String,
    pub change: Change,
    pub installed: bool,
    pub held: bool,
    pub steps: Vec<StepChange>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionPreview {
    pub revisions: ReviewedRevisions,
    pub current_tags: Vec<String>,
    pub candidate_tags: Vec<String>,
    pub reference: String,
    pub apps: Vec<AppChange>,
    pub other_files: Vec<String>,
}

impl Collection {
    pub fn preview_update(&self, requested: Option<&crate::settings::Tracking>) -> Result<CollectionPreview> {
        let mut preview = if self.managed && crate::service::enabled() {
            // Only the service fetches into its clone and resolves machine policy. It includes
            // its own installation records; the client adds installations made for this user.
            ensure!(requested.is_none(), "'tracking' is set by your organization for collection '{}'", self.name);
            crate::service::preview_collection(&self.name)?
        } else {
            let _lock = self.lock()?;
            let url = origin_url(&self.dir)?;
            let config = crate::settings::for_collection(&self.name).settings;
            let tracking = crate::settings::tracking_for(&self.name, &url, self.managed, requested)?;
            let target = self.verified_target(&url, &tracking, false, config.checkout_depth)?;
            let revisions = ReviewedRevisions { current: self.head()?, candidate: target.commit };
            let (apps, other_files) = self.compare_apps(&revisions)?;
            // An initial shallow checkout can have no local tag refs. Ask the same remote
            // for tags that currently name these commits, including annotated (peeled) tags.
            let tags = git_bytes(Some(&self.dir), &["ls-remote", "--tags", "--end-of-options", "origin"])?;
            CollectionPreview {
                current_tags: tags_for(&tags, &revisions.current),
                candidate_tags: tags_for(&tags, &revisions.candidate),
                revisions,
                reference: target.reference,
                apps,
                other_files,
            }
        };
        let dir = data_dir();
        let installed = crate::options::recorded_apps(&dir);
        let versions = crate::versions::all(&dir);
        let choose_version = crate::settings::for_collection(&self.name).settings.choose_version;
        for app in &mut preview.apps {
            let key = format!("{}/{}", self.name, app.id);
            let record = versions.get(&key);
            app.installed |= installed.contains(&key) || record.is_some_and(|r| r.good.is_some());
            if let Some(held) = record.and_then(|r| r.held.as_ref()) {
                let blob = self.blob(&preview.revisions.current, &format!("apps/{}.dsc.yaml", app.id));
                app.held = hold_applies(held, blob.as_deref(), choose_version);
            }
        }
        Ok(preview)
    }

    /// Collect the file versions without hashing or opening files in the working tree.
    fn compare_apps(&self, revisions: &ReviewedRevisions) -> Result<(Vec<AppChange>, Vec<String>)> {
        let diff = git_bytes(
            Some(&self.dir),
            &[
                "diff-tree",
                "--no-commit-id",
                "-r",
                "--no-renames",
                "--raw",
                "--no-abbrev",
                "-z",
                &revisions.current,
                &revisions.candidate,
                "--",
            ],
        )?;
        let mut items = diff.split(|b| *b == 0);
        let mut definitions = BTreeMap::new();
        let mut other = BTreeSet::new();
        while let Some(raw) = items.next().filter(|b| !b.is_empty()) {
            let path = items.next().context("incomplete Git tree diff")?;
            let Some(id) = app_id(path)? else {
                other.insert(String::from_utf8_lossy(path).into_owned());
                continue;
            };
            let header: Vec<_> = std::str::from_utf8(raw)?.trim_start_matches(':').split_ascii_whitespace().collect();
            ensure!(header.len() == 5, "invalid Git diff entry for {id}");
            let blob = |mode: &str, oid: &str| -> Result<Option<String>> {
                if mode == "000000" {
                    return Ok(None);
                }
                ensure!(
                    matches!(mode, "100644" | "100755"),
                    "cannot compare apps/{id}.dsc.yaml: expected a regular file"
                );
                Ok(Some(oid.into()))
            };
            definitions.insert(id.to_owned(), (blob(header[0], header[2])?, blob(header[1], header[3])?, true));
        }
        if !other.is_empty() {
            // An unchanged app can use a changed script, installer or configuration template.
            let tree = git_bytes(Some(&self.dir), &["ls-tree", "-r", "-z", &revisions.candidate, "--", "apps"])?;
            for entry in tree.split(|b| *b == 0).filter(|e| !e.is_empty()) {
                let tab = entry.iter().position(|b| *b == b'\t').context("invalid Git tree entry")?;
                let Some(id) = app_id(&entry[tab + 1..])? else {
                    continue;
                };
                if definitions.contains_key(id) {
                    continue;
                }
                let header: Vec<_> = std::str::from_utf8(&entry[..tab])?.split_ascii_whitespace().collect();
                ensure!(
                    header.len() == 3 && matches!(header[0], "100644" | "100755"),
                    "cannot compare apps/{id}.dsc.yaml: expected a regular file"
                );
                definitions.insert(id.into(), (Some(header[2].into()), Some(header[2].into()), false));
            }
        }
        let ids: BTreeSet<_> = definitions
            .values()
            .flat_map(|(old, new, _)| [old.as_deref(), new.as_deref()].into_iter().flatten())
            .collect();
        let blobs = self.blobs(&ids.into_iter().collect::<Vec<_>>());
        let mut apps = Vec::new();
        for (id, (old, new, definition_changed)) in definitions {
            let load = |oid: Option<&str>| -> Result<Vec<Declared>> {
                let Some(oid) = oid else { return Ok(vec![]) };
                let bytes = blobs.get(oid).with_context(|| format!("could not read apps/{id}.dsc.yaml at {oid}"))?;
                declarations(bytes).with_context(|| format!("cannot compare apps/{id}.dsc.yaml at {oid}"))
            };
            let change = match (&old, &new) {
                (None, _) => Change::Added,
                (_, None) => Change::Removed,
                _ => Change::Changed,
            };
            let new_steps = load(new.as_deref())?;
            let steps = if definition_changed {
                changed_steps(load(old.as_deref())?, new_steps, &other)
            } else {
                new_steps.iter().filter(|s| s.uses(&other)).map(|s| s.change(Change::Changed)).collect()
            };
            if definition_changed || !steps.is_empty() {
                apps.push(AppChange { id, change, installed: false, held: false, steps });
            }
        }
        Ok((apps, other.into_iter().collect()))
    }
}

fn app_id(path: &[u8]) -> Result<Option<&str>> {
    path.strip_prefix(b"apps/")
        .and_then(|p| p.strip_suffix(b".dsc.yaml"))
        .filter(|id| !id.contains(&b'/'))
        .map(std::str::from_utf8)
        .transpose()
        .context("app identifier is not UTF-8")
}

fn tags_for(refs: &[u8], commit: &str) -> Vec<String> {
    refs.split(|b| *b == b'\n')
        .filter_map(|line| std::str::from_utf8(line).ok())
        .filter_map(|line| line.split_once('\t'))
        .filter(|(oid, _)| *oid == commit)
        .filter_map(|(_, reference)| reference.strip_prefix("refs/tags/"))
        .map(|tag| tag.trim_end_matches("^{}").to_owned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

type Identity = (bool, Vec<(String, String, usize)>);

struct Declared {
    key: Identity,
    name: String,
    kind: String,
    body: Value,
}

impl Declared {
    fn change(&self, change: Change) -> StepChange {
        StepChange { name: self.name.clone(), kind: self.kind.clone(), removal: self.key.0, change }
    }

    fn uses(&self, changed: &BTreeSet<String>) -> bool {
        let field = if self.kind.eq_ignore_ascii_case(super::INSTALLER_TYPE) {
            "file"
        } else if [super::FILE_TYPE, super::COMMAND_TYPE, super::DIRECTORY_TYPE].iter().any(|kind| self.kind.eq_ignore_ascii_case(kind)) {
            "source"
        } else {
            return false;
        };
        let Some(source) = self.body["properties"][field].as_str() else {
            return false;
        };
        let path: std::path::PathBuf = std::path::Path::new(source).components().collect();
        let normalized = path.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
        if self.kind.eq_ignore_ascii_case(super::DIRECTORY_TYPE) {
            let prefix = format!("{normalized}/");
            return changed.iter().any(|path| path.starts_with(&prefix));
        }
        changed.contains(&normalized)
    }
}

fn declarations(bytes: &[u8]) -> Result<Vec<Declared>> {
    // Never include the collection's option or resource values in output or parse errors.
    let mut doc: Value = serde_yaml::from_slice(bytes).map_err(|_| anyhow::anyhow!("invalid DSC document"))?;
    let mut steps = Vec::new();
    let mut collect = |steps_doc: &mut Value, removal: bool| -> Result<()> {
        let list = steps_doc.as_array_mut().context("expected a list of DSC resources")?;
        flatten(list, &(removal, Vec::new()), &mut steps)
    };
    collect(doc.get_mut("resources").context("missing DSC resources")?, false)?;
    if let Some(uninstall) = doc.pointer_mut("/metadata/furnish/uninstall") {
        collect(uninstall, true)?;
    }
    Ok(steps)
}

fn flatten(resources: &mut [Value], parent: &Identity, steps: &mut Vec<Declared>) -> Result<()> {
    let mut occurrences = BTreeMap::new();
    for item in resources {
        let name = item["name"].as_str().context("missing DSC resource name")?.to_owned();
        let kind = item["type"].as_str().context("missing DSC resource type")?.to_owned();
        let identity = (name, kind.to_ascii_lowercase());
        let index = occurrences.entry(identity.clone()).or_insert(0);
        let mut key = parent.clone();
        key.1.push((identity.0, identity.1, *index));
        *index += 1;
        let mut children = item.pointer_mut("/properties/resources").filter(|r| r.is_array()).map(Value::take);
        let name = key.1.iter().map(|(n, _, _)| n.as_str()).collect::<Vec<_>>().join(" › ");
        steps.push(Declared { key: key.clone(), name, kind, body: item.take() });
        if let Some(children) = children.as_mut().and_then(Value::as_array_mut) {
            flatten(children, &key, steps)?;
        }
    }
    Ok(())
}

fn changed_steps(old: Vec<Declared>, new: Vec<Declared>, files: &BTreeSet<String>) -> Vec<StepChange> {
    let old_keys: BTreeSet<_> = old.iter().map(|s| &s.key).collect();
    let new_keys: BTreeSet<_> = new.iter().map(|s| &s.key).collect();
    let old_rank: BTreeMap<_, _> =
        old.iter().filter(|s| new_keys.contains(&s.key)).enumerate().map(|(i, s)| (&s.key, i)).collect();
    let previous: BTreeMap<_, _> = old.iter().map(|s| (&s.key, &s.body)).collect();
    let mut changes = Vec::new();
    let mut add = |s: &Declared, change| changes.push(s.change(change));
    let mut rank = 0;
    for step in &new {
        if !old_keys.contains(&step.key) {
            add(step, Change::Added);
        } else {
            if previous[&step.key] != &step.body || old_rank[&step.key] != rank || step.uses(files) {
                add(step, Change::Changed);
            }
            rank += 1;
        }
    }
    for step in &old {
        if !new_keys.contains(&step.key) {
            add(step, Change::Removed);
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_steps_and_explicit_removals_keep_their_own_identities() {
        let old = br#"
resources:
  - name: Configure
    type: Microsoft.DSC/Group
    properties:
      resources: [{name: Tool, type: Furnish/Package, properties: {name: tool}}]
metadata:
  furnish:
    uninstall: [{name: Tool, type: Furnish/Package, properties: {_exist: false, name: tool}}]
"#;
        let new = std::str::from_utf8(old).unwrap().replace("name: tool}}]", "name: new-tool}}]");
        let changes =
            changed_steps(declarations(old).unwrap(), declarations(new.as_bytes()).unwrap(), &BTreeSet::new());
        assert_eq!(changes.len(), 2);
        assert_eq!(
            (changes[0].name.as_str(), changes[0].removal, changes[0].change),
            ("Configure › Tool", false, Change::Changed)
        );
        assert_eq!((changes[1].name.as_str(), changes[1].removal, changes[1].change), ("Tool", true, Change::Changed));
    }

    #[test]
    fn directory_source_matches_descendants_only() {
        let doc = b"resources: [{name: Docs, type: Furnish/Directory, properties: {source: docs/offline/}}]";
        let steps = declarations(doc).unwrap();
        assert!(steps[0].uses(&["docs/offline/chapter/index.html".to_owned()].into()));
        assert!(!steps[0].uses(&["docs/offline-old/index.html".to_owned()].into()));
    }

    #[test]
    fn malformed_candidate_is_an_error_without_echoing_property_values() {
        for document in ["token-value", "resources: token-value", "resources: [token-value]", "resources: ["] {
            let Err(err) = declarations(document.as_bytes()) else { panic!("unexpected success") };
            assert!(!format!("{err:#}").contains("token-value"));
        }
    }
}
