//! Look up an installed package without asking `nix profile` to initialize the user's
//! default profile. The v1/v2 manifests list elements; v3 keys them by profile entry name.
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::Value;

pub(crate) fn version(manifest: &Path, name: &str) -> Result<String> {
    let bytes = std::fs::read(manifest).with_context(|| format!("reading {}", manifest.display()))?;
    let state = serde_json::from_slice(&bytes).with_context(|| format!("reading {}", manifest.display()))?;
    Ok(installed(&state, name)?.unwrap_or_default().to_owned())
}

fn installed<'a>(state: &'a Value, name: &str) -> Result<Option<&'a str>> {
    let entry = name.rsplit('.').next().unwrap_or(name);
    let element = match (state["version"].as_u64(), &state["elements"]) {
        (Some(1..=3), Value::Object(elements)) => elements.get(entry),
        (Some(1..=3), Value::Array(elements)) => elements.iter().find(|e| match e["attrPath"].as_str() {
            Some(attr) => attr == name || attr.strip_suffix(name).is_some_and(|prefix| prefix.ends_with('.')),
            None => store_names(e).any(|s| name_version(s).0 == entry),
        }),
        _ => bail!("unrecognized Nix profile manifest"),
    };
    let Some(e) = element else { return Ok(None) };
    Ok(Some(store_names(e).find_map(|s| name_version(s).1).unwrap_or("?")))
}

// The attribute and pname can differ. Nix store names use <pname>-<version>,
// optionally followed by an output name such as -bin or -dev.
fn name_version(s: &str) -> (&str, Option<&str>) {
    let Some(dash) = s.as_bytes().windows(2).position(|pair| pair[0] == b'-' && pair[1].is_ascii_digit()) else {
        return (s, None);
    };
    let version = &s[dash + 1..];
    let version = match version.rsplit_once('-') {
        Some((v, "bin" | "dev" | "lib" | "man" | "doc" | "out" | "info" | "data")) => v,
        _ => version,
    };
    (&s[..dash], Some(version))
}

fn store_names(e: &Value) -> impl Iterator<Item = &str> {
    e["storePaths"].as_array().into_iter().flatten().filter_map(|p| {
        Path::new(p.as_str()?).file_name()?.to_str()?.split_once('-').map(|(_, name)| name)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn finds_the_version_only_in_the_selected_profile_element() {
        let paths = json!(["/nix/store/hash-hello-world-2.1-bin"]);
        let current = json!({"version":3, "elements":{"tool":{"storePaths":paths},"unrelated":{"storePaths":["/nix/store/hash-tool-8"]}}});
        assert_eq!(installed(&current, "tools.tool").unwrap(), Some("2.1"));
        assert_eq!(installed(&current, "hello-world").unwrap(), None);
        for version in [1, 2] {
            let state = json!({"version":version, "elements":[{"attrPath":"legacyPackages.x86_64-linux.tools.tool","storePaths":paths},
                {"storePaths":["/nix/store/hash-noattr-3"]}]});
            assert_eq!(installed(&state, "tools.tool").unwrap(), Some("2.1"));
            assert_eq!(installed(&state, "noattr").unwrap(), Some("3"));
            assert_eq!(installed(&state, "other.tool").unwrap(), None);
        }
        assert_eq!(installed(&json!({"version":3,"elements":{"x":{"storePaths":["/nix/store/hash-unknown"]}}}), "x").unwrap(), Some("?"));
    }
}
