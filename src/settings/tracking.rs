//! The Git reference a collection follows and who selected it.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::policy::Policy;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "camelCase")]
pub enum TagOrder {
    #[default]
    Version,
    Newest,
}
impl TagOrder {
    fn is_version(&self) -> bool {
        *self == Self::Version
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(tag = "mode", rename_all = "camelCase", deny_unknown_fields)]
pub enum Tracking {
    #[default]
    Default,
    Branch {
        name: String,
    },
    Tag {
        pattern: String,
        #[serde(skip_serializing_if = "TagOrder::is_version")]
        order: TagOrder,
    },
}

impl<'de> Deserialize<'de> for Tracking {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(tag = "mode", rename_all = "camelCase", deny_unknown_fields)]
        enum Selector {
            Default {},
            Branch {
                name: String,
            },
            Tag {
                pattern: String,
                #[serde(default)]
                order: TagOrder,
            },
        }
        let tracking = match Selector::deserialize(deserializer)? {
            Selector::Default {} => Self::Default,
            Selector::Branch { name } => Self::Branch { name },
            Selector::Tag { pattern, order } => Self::Tag { pattern, order },
        };
        tracking.validate().map_err(serde::de::Error::custom)?;
        Ok(tracking)
    }
}

impl Tracking {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Default => {}
            Self::Branch { name } => {
                // Rules of git-check-ref-format for literal local branch names. Never accept
                // previous-checkout syntax or an option where an explicit branch is expected.
                ensure!(
                    !name.is_empty()
                        && !name.starts_with('-')
                        && name != "HEAD"
                        && !name.ends_with('.')
                        && !name.contains("..")
                        && !name.contains("@{")
                        && !name.bytes().any(|b| b <= b' ' || b == 127 || b"~^:?*[\\".contains(&b))
                        && name.split('/').all(|p| !p.is_empty() && !p.starts_with('.') && !p.ends_with(".lock")),
                    "invalid Git branch name '{name}'"
                );
            }
            Self::Tag { pattern, .. } => {
                ensure!(!pattern.trim().is_empty(), "tag pattern must not be empty");
                regex::Regex::new(pattern).context("invalid tag regular expression")?;
            }
        }
        Ok(())
    }

    pub(crate) fn parse(value: &Value) -> Result<Self> {
        serde_json::from_value(value.clone()).context("invalid collection tracking selector")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TrackingSource {
    Default,
    User,
    Policy,
    Endpoint,
}

pub(super) struct EffectiveTracking {
    pub value: Tracking,
    pub source: TrackingSource,
    pub locked: bool,
}

pub(super) fn resolve(
    policy: &Policy,
    choices: &Map<String, Value>,
    name: &str,
    url: &str,
    managed: bool,
    change: Option<Option<&Tracking>>,
) -> Result<EffectiveTracking> {
    let preset = policy.collections.preset.iter().find(|p| p.name == name);
    if managed {
        ensure!(preset.is_some_and(|p| p.url == url), "collection '{name}' has no matching managed origin");
    }
    let preset = preset.filter(|p| p.url == url);
    let machine = policy.collection_settings.get(name).and_then(|c| c.get("tracking"));
    let locked = managed || preset.is_some() || machine.is_some_and(|e| e.locked);
    if change.is_some() && locked {
        bail!("'tracking' is set by your organization for collection '{name}'");
    }

    let parse = |v: &Value, source| -> Result<EffectiveTracking> {
        let value = Tracking::parse(v).with_context(|| format!("invalid tracking for collection '{name}'"))?;
        Ok(EffectiveTracking { value, source, locked })
    };
    if let Some(value) = preset.and_then(|p| p.tracking.as_ref()) {
        return parse(value, TrackingSource::Policy);
    }
    if !locked {
        match change {
            Some(Some(requested)) => {
                requested.validate()?;
                return Ok(EffectiveTracking {
                    value: requested.clone(),
                    source: TrackingSource::User,
                    locked,
                });
            }
            None => {
                if let Some(mine) = choices.get("collections").and_then(|c| c.get(name)).and_then(|c| c.get("tracking")) {
                    return parse(mine, TrackingSource::User);
                }
            }
            Some(None) => {}
        }
    }
    if let Some(entry) = machine {
        return parse(&entry.value, TrackingSource::Policy);
    }
    if let Some(setting) = policy.endpoint_tracking.get(&(name.to_string(), url.to_string())) {
        return Ok(EffectiveTracking {
            value: setting.clone(),
            source: TrackingSource::Endpoint,
            locked,
        });
    }
    Ok(EffectiveTracking {
        value: Tracking::Default,
        source: TrackingSource::Default,
        locked,
    })
}

/// Resolve against the actual remote URL, including before a first clone exists.
pub fn tracking_for(name: &str, url: &str, managed: bool, requested: Option<&Tracking>) -> Result<Tracking> {
    let choices = if managed { Map::new() } else { super::read_choices()? };
    Ok(resolve(&crate::policy::load(), &choices, name, url, managed, requested.map(Some))?.value)
}

/// Preview the effective selector after a change. None removes the personal override.
pub fn tracking_for_change(name: &str, url: &str, managed: bool, requested: Option<&Tracking>) -> Result<Tracking> {
    Ok(resolve(&crate::policy::load(), &Map::new(), name, url, managed, Some(requested))?.value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn wire_format_rejects_malformed_and_ambiguous_selectors() {
        for selector in [
            json!({"mode":"default", "name":"stable"}),
            json!({"mode":"tag","pattern":" "}),
            json!({"mode":"tag","pattern":"["}),
            json!({"mode":"branch","name":"-main"}),
            json!({"mode":"branch","name":"feature/../x"}),
            json!({"mode":"branch","name":"foo.lock/test"}),
            json!({"mode":"branch","name":"@{-1}"}),
            json!({"mode":"branch","name":""}),
            json!({"mode":"tag","pattern":"v","order":"largest"}),
            json!({"mode":"future"}),
            Value::Null,
        ] {
            assert!(Tracking::parse(&selector).is_err(), "{selector}");
        }
        for selector in [
            json!({"mode":"default"}),
            json!({"mode":"tag","pattern":"^v[0-9]+"}),
            json!({"mode":"tag","pattern":"^r","order":"newest"}),
            json!({"mode":"branch","name":"release/1.2"}),
        ] {
            assert_eq!(serde_json::to_value(Tracking::parse(&selector).unwrap()).unwrap(), selector);
        }
    }

    #[test]
    fn endpoint_default_matches_origin_and_personal_choices_can_be_reset() {
        let url = "https://org.test/git/tools";
        let mut policy = Policy::default();
        let tag = Tracking::Tag {
            pattern: "^release-".into(),
            order: TagOrder::Version,
        };
        policy.endpoint_tracking.insert(("tools".into(), url.into()), tag.clone());
        let mine = json!({"collections":{"tools":{"tracking":{"mode":"branch","name":"mine"}}}});
        let choices = mine.as_object().unwrap();
        assert_eq!(resolve(&policy, choices, "tools", url, false, None).unwrap().source, TrackingSource::User);
        let reset = resolve(&policy, choices, "tools", url, false, Some(None)).unwrap();
        assert_eq!((reset.value, reset.source, reset.locked), (tag, TrackingSource::Endpoint, false));
        assert_eq!(
            resolve(&policy, choices, "tools", "other", false, Some(None)).unwrap().value,
            Tracking::Default
        );
    }

    #[test]
    fn policy_locks_and_managed_origins_ignore_personal_overrides() {
        let uri = "https://org.test/git/tools";
        let json = json!({"collections":{"preset":[{"name":"tools","url":uri}]},"collectionSettings":{"tools":{
            "tracking":{"value":{"mode":"branch","name":"stable"},"locked":false}}}});
        let mut policy: Policy = serde_json::from_value(json).unwrap();
        let mine = json!({"collections":{"tools":{"tracking":{"mode":"branch","name":"mine"}}}});
        let choices = mine.as_object().unwrap();
        let get = |p: &Policy| resolve(p, choices, "tools", uri, true, None).unwrap();
        let actual = get(&policy);
        assert_eq!(
            (actual.value, actual.source, actual.locked),
            (Tracking::Branch { name: "stable".into() }, TrackingSource::Policy, true)
        );
        policy.collections.preset[0].tracking = Some(json!({"mode":"default"}));
        assert_eq!(get(&policy).value, Tracking::Default);
        assert!(resolve(&policy, choices, "tools", uri, true, Some(None)).is_err());
        assert!(resolve(&policy, choices, "tools", "different-url", true, None).is_err());
        policy.collections.preset[0].tracking = Some(json!({"mode":"tag","pattern":"["}));
        assert!(resolve(&policy, choices, "tools", uri, true, None).is_err());
        policy.collections.preset.clear();
        policy.collection_settings.get_mut("tools").unwrap().get_mut("tracking").unwrap().locked = true;
        assert!(resolve(&policy, choices, "tools", uri, false, Some(None)).is_err());
        assert_eq!(
            resolve(&policy, choices, "tools", uri, false, None).unwrap().source,
            TrackingSource::Policy
        );
    }
}
