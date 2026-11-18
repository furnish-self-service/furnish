//! Drive the collection-author examples through the installed CLI entry point.
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

fn check(root: &Path, collection: &Path, profiles: Option<&Path>, success: bool) -> String {
    check_env(root, collection, profiles, success, None)
}

fn check_env(root: &Path, collection: &Path, profiles: Option<&Path>, success: bool, role: Option<&str>) -> String {
    let mut command = Command::new(env!("CARGO_BIN_EXE_furnish"));
    command
        .arg("check")
        .arg(collection)
        .env("HOME", root)
        .env("FURNISH_DATA_DIR", root.join("data"))
        .env("FURNISH_SYSTEM_DIR", root.join("system"))
        .env("FURNISH_POLICY_FILE", root.join("policy.json"));
    command.env_remove("FURNISH_TEST_PROFILE_ROLE");
    if let Some(role) = role {
        command.env("FURNISH_TEST_PROFILE_ROLE", role);
    }
    if let Some(path) = profiles {
        command.arg("--profiles").arg(path);
    }
    let out = command.output().unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert_eq!(out.status.code(), Some(if success { 0 } else { 1 }), "{text}");
    text
}

fn app(collection: &Path, name: &str, doc: Value) {
    let apps = collection.join("apps");
    fs::create_dir_all(&apps).unwrap();
    fs::write(apps.join(format!("{name}.dsc.yaml")), doc.to_string()).unwrap();
}

fn package(name: &str) -> Value {
    json!({"name":name, "type":"Furnish/Package", "properties":{"name":name}})
}

#[test]
fn documented_windows_and_arch_matrix_and_failed_assertion() {
    let temp = tempfile::tempdir().unwrap();
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR"));
    let collection = checkout.join("tests/fixtures/collection");
    let path = checkout.join("docs/examples/target-profiles.yaml");
    let output = check(temp.path(), &collection, Some(&path), true);
    for line in ["arch-workstation", "windows-office", "workstation-tool", "sharkdp.hyperfine", "skipped hyperfine (winget)", "is false)"] {
        assert!(output.contains(line), "{output}");
    }
    let broken = temp.path().join("mismatch.yaml");
    fs::write(&broken, fs::read_to_string(path).unwrap().replace("manager: pacman, name:", "manager: nix, name:")).unwrap();
    let output = check(temp.path(), &collection, Some(&broken), false);
    assert!(output.contains("renamed: expected packages") && output.contains("got "), "{output}");
}

#[test]
fn documented_editor_checks_options_dependencies_and_ordered_distribution_like() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("repo");
    app(&dir, "developer-tools", json!({"resources":[package("tools")]}));
    app(
        &dir,
        "editor",
        json!({
            "parameters":{"plugins":{"type":"bool","defaultValue":false},"role":{"type":"string","defaultValue":"office"}},
            "variables":{"archEditor":"[concat('co', 'de')]"},
            "metadata":{"furnish":{"dependencies":[{"app":"developer-tools","condition":"[and(parameters('plugins'), equals(parameters('role'), 'developer'))]"}]}},
            "resources":[
                {"name":"Install on Windows","type":"Furnish/Package","condition":"[equals(context().os.family, 'Windows')]","properties":{"name":"editor"}},
                {"name":"Editor","type":"Furnish/Package","properties":{"name":"editor","namesByDistribution":{"arch":"[variables('archEditor')]"}}}
            ]
        }),
    );
    fs::write(dir.join("collection.yaml"), "parameters: {role: {type: string, default: office}}").unwrap();
    let guide = include_str!("../docs/site/collections.md");
    let yaml = guide.split("### Check target profiles in CI").nth(1).unwrap().split("```yaml\n").nth(1).unwrap().split("```").next().unwrap();
    let path = dir.join("targets.yaml");
    fs::write(&path, yaml).unwrap();
    let output = check(temp.path(), &dir, Some(&path), true);
    assert!(output.contains("editor: active dependencies: developer-tools"), "{output}");
}

#[test]
fn mutually_exclusive_edges_pass_and_a_concrete_cycle_fails() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("repo");
    let doc = |name, dependency, condition: &str| {
        json!({
            "metadata":{"furnish":{"dependencies":[{"app":dependency,"condition":condition}]}},
            "resources":[package(name)]
        })
    };
    app(&dir, "a", doc("a", "b", "[equals(context().os.family, 'Linux')]"));
    app(&dir, "b", doc("b", "a", "[equals(context().os.family, 'Windows')]"));
    let output = check(temp.path(), &dir, None, true);
    assert!(!output.contains("form a cycle"), "{output}");

    let distro = "[equals(variables('furnish.host').distribution, 'alpine')]";
    app(&dir, "b", doc("b", "a", distro));
    let path = dir.join("targets.yaml");
    fs::write(&path, "profiles: {ubuntu: {family: Linux, distribution: ubuntu, packageManagers: [apt]}}").unwrap();
    let output = check(temp.path(), &dir, Some(&path), true);
    assert!(output.contains("may form a cycle on Linux"), "{output}");
    fs::write(&path, "profiles: {alpine: {family: Linux, distribution: alpine, packageManagers: []}}").unwrap();
    let output = check(temp.path(), &dir, Some(&path), false);
    assert!(output.contains("error: dependencies form a cycle on alpine: a → b → a"), "{output}");
}

#[test]
fn profile_names_missing_prerequisites_and_refuses_live_machine_facts() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("repo");
    let path = temp.path().join("profile.yaml");
    fs::write(&path, "profiles: {workstation: {family: Linux, packageManagers: [pacman]}}").unwrap();
    let mut step = package("tool");
    step["properties"]["packageManager"] = "brew".into();
    app(&dir, "tool", json!({"resources":[step]}));
    let report = check(temp.path(), &dir, Some(&path), true);
    assert!(report.contains("missing prerequisite (package manager brew) for tool"), "{report}");
    for (expression, reason) in [
        ("[equals(envvar('PATH'), 'absent')]", "envvar() requires facts outside the target profile"),
        ("[equals(tryWhich('git'), '')]", "tryWhich() requires facts outside the target profile"),
        ("[contains(context().os, 'version')]", "only context().os.family is defined by the target profile"),
    ] {
        step["condition"] = expression.into();
        app(&dir, "tool", json!({"resources":[step]}));
        let report = check(temp.path(), &dir, Some(&path), false);
        assert!(report.contains(reason), "{report}");
    }
}

#[test]
fn profile_script_selection_respects_platform_and_real_native_priority() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("repo");
    let path = temp.path().join("profile.yaml");
    let mut step = package("tool");
    step["properties"]["namesByManager"] = json!({"custom":"script-tool"});
    app(&dir, "tool", json!({"resources":[step]}));
    fs::write(dir.join("manager.sh"), "exit 99").unwrap();
    fs::write(dir.join("collection.yaml"), "packageManagers: {custom: {script: manager.sh}}").unwrap();
    fs::write(&path, "profiles: {portable: {family: Linux, packageManagers: [custom], expect: {tool: {packages: {tool: {manager: custom, name: script-tool}}}}}}").unwrap();
    check(temp.path(), &dir, Some(&path), true);
    for (family, ids, diagnostic) in [
        ("Windows", "custom", "needs an interpreter for manager.sh on Windows"),
        ("Linux", "apt, pacman", "must follow Furnish priority: pacman, apt"),
    ] {
        fs::write(&path, format!("profiles: {{invalid: {{family: {family}, packageManagers: [{ids}]}}}}")).unwrap();
        let report = check(temp.path(), &dir, Some(&path), false);
        assert!(report.contains(diagnostic), "{report}");
    }
}

#[test]
fn family_preview_is_identical_with_or_without_ci_environment_and_validates_defaults() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("repo");
    let mut doc = json!({
        "parameters":{
            "role":{"type":"string","defaultValue":"[envvar('FURNISH_TEST_PROFILE_ROLE')]"},
            "windows":{"type":"bool","defaultValue":"[equals(context().os.family, 'Windows')]"}
        },
        "variables":{"localRole":"[envvar('FURNISH_TEST_PROFILE_ROLE')]"},
        "metadata":{"furnish":{"recommended":"[equals(parameters('role'), 'developer')]"}},
        "resources":[
            {"name":"environment","condition":"[equals(parameters('role'), 'developer')]","type":"Furnish/Package","properties":{"name":"tool"}},
            {"name":"windows","condition":"[parameters('windows')]","type":"Furnish/Package","properties":{"name":"tool"}}
        ]
    });
    app(&dir, "tool", doc.clone());
    let without = check_env(temp.path(), &dir, None, true, None);
    let with = check_env(temp.path(), &dir, None, true, Some("developer"));
    assert_eq!(with, without);
    assert!(with.contains("Windows: environment (depends on host), windows") && with.contains("Linux: environment (depends on host)"), "{with}");
    for (invalid, needle) in [
        ("[envvar(123)]", "argument 1 does not accept Number"),
        ("[spelling(envvar('FURNISH_TEST_PROFILE_ROLE'))]", "unknown DSC function spelling"),
        ("[false()]", "error:"),
    ] {
        doc["parameters"]["role"]["defaultValue"] = invalid.into();
        app(&dir, "tool", doc.clone());
        let output = check_env(temp.path(), &dir, None, false, None);
        assert!(output.contains(needle), "{output}");
    }
}

#[test]
fn only_if_set_up_skips_windows_and_identifies_unmodeled_unix_setup() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("repo");
    let path = temp.path().join("targets.yaml");
    app(&dir, "tool", json!({"resources":[{"name":"desktop","type":"Furnish/NixOS","properties":{"name":"desktop","onlyIfSetUp":true}}]}));
    fs::write(&path, "profiles: {office: {family: Windows, packageManagers: [winget], expect: {tool: {steps: [], skipped: [desktop]}}}}")
        .unwrap();
    let output = check(temp.path(), &dir, Some(&path), true);
    assert!(output.contains("skipped desktop") && output.contains("Linux: desktop (depends on host)"), "{output}");
    fs::write(&path, "profiles: {dev: {family: Linux, packageManagers: [nix], expect: {tool: {steps: [desktop]}}}}").unwrap();
    let output = check(temp.path(), &dir, Some(&path), false);
    assert!(output.contains("onlyIfSetUp requires the target's NixOS or Home Manager configuration"), "{output}");
}
