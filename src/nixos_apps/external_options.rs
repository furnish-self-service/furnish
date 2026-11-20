//! Inspect option-only declarations in the selected flake before claiming a new apps.json entry.
use super::*;

/// Query only the requested attributes. Quoting each validated component also permits Nix
/// keywords as names. Missing attributes fail evaluation, even when the expected value is null.
fn projection(options: &BTreeMap<String, Value>) -> Option<String> {
    if options.is_empty() || !options.keys().all(|key| plain_name(key, "")) {
        return None;
    }
    let mut expr = String::from("config: [");
    for key in options.keys() {
        expr.push_str(" (config");
        for part in key.split('.') {
            expr.push_str(&format!(".\"{part}\""));
        }
        expr.push(')');
    }
    expr.push_str(" ]");
    Some(expr)
}

pub(super) fn probe(desired: &Desired, person: &Account) -> Option<(Place, PathBuf)> {
    if !desired.packages.is_empty() || (desired.options.is_empty() && desired.home_options.is_empty()) {
        return None;
    }
    let place = place_for(desired, person);
    let options = if place == Place::System { &desired.options } else { &desired.home_options };
    let expr = projection(options)?;
    let (dir, config) = match place {
        Place::HomeStandalone => {
            let facts = home_facts(person).unwrap_or_default();
            let dir = home_dir_of(person, &facts);
            let attr = facts.nix.flake_attribute.as_deref().unwrap_or(&person.name);
            (dir, format!("homeConfigurations.\"{attr}\".config"))
        }
        Place::System | Place::HomeNixos => {
            let setup = crate::nixos::detect(&crate::nixos::Context::new(in_service()))?;
            let found = setup.configuration.filter(|c| c.kind == crate::nixos::Kind::Flake)?;
            let mut config = format!("nixosConfigurations.\"{}\".config", setup.flake_attribute.value);
            if place == Place::HomeNixos {
                config.push_str(&format!(".home-manager.users.\"{}\"", person.name));
            }
            (found.path, config)
        }
    };
    if !dir.is_absolute() || !dir.join("flake.nix").is_file() {
        return None;
    }
    // A service lookup evaluates as the requesting user. For root's own request, apply the
    // rebuild ownership checks before running Nix in a repository and use the folder's owner.
    let mut owner = None;
    let mut trust = None;
    if is_root() && person.uid == 0 {
        let uid = may_rebuild(&chain(&dir).ok()?, 0, &[], group_name).ok()?;
        owner = Some(lookup(uid)?);
        trust = GitTrust::new(&dir, owner.as_ref()?, &[]).ok()?;
    }
    let mut cmd = nix(owner.as_ref().unwrap_or(person));
    if let Some(t) = &trust {
        t.apply(&mut cmd);
    }
    // A status check doesn't build derivations or update the user's lock. If the flake
    // can't be evaluated, leave the normal install and its diagnostics available.
    let out = cmd.current_dir(&dir)
        .args(["eval", "--json", "--offline", "--no-update-lock-file", "--no-write-lock-file",
            "--option", "allow-import-from-derivation", "false", "--option", "accept-flake-config", "false"])
        .arg(format!(".#{config}")).arg("--apply").arg(expr)
        .stderr(Stdio::null()).output().ok()?;
    if !out.status.success() { return None }
    let actual: Vec<Value> = serde_json::from_slice(&out.stdout).ok()?;
    actual.iter().eq(options.values()).then_some((place, dir))
}

pub(super) fn response(desired: &Desired, place: Place, dir: &Path, action: &str, what_if: bool) -> Result<Value> {
    let mut actual = state(&desired.name, Some(&(place, desired.entry(place))));
    actual["managedBy"] = if place == Place::System { "nixos" } else { "home-manager" }.into();
    actual["declaredIn"] = json!(dir);
    let note = format!("{} is already set in {} ({})", desired.name, place.what(), dir.display());
    match action {
        "get" => {}
        "test" => actual["_inDesiredState"] = desired.exist.into(),
        "set" if what_if => actual["_metadata"] = json!({"whatIf":[note], "furnishChange":false}),
        "set" if desired.exist => {}
        "set" => bail!("{note}: remove its options there"),
        _ => bail!("usage: get|test|set [--what-if]"),
    }
    Ok(actual)
}
