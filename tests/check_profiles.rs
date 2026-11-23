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
fn profile_uses_its_own_documents_directory_in_parameter_defaults() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("repo");
    let path = temp.path().join("targets.yaml");
    let location = "/home/alice/Dokumente/Team docs";
    app(&dir, "docs", json!({
        "parameters":{"location":{"type":"string","defaultValue":"[concat(variables('furnish.host').documentsDir, '/Team docs')]"}},
        "resources":[{"name":"team", "type":"Furnish/Directory", "condition":format!("[equals(parameters('location'), '{location}')]"),
            "properties":{"path":"[parameters('location')]"}}]
    }));
    let target = |documents: &str, steps: &[&str]| json!({
        "family":"Linux", "packageManagers":[], "documentsDir":documents, "expect":{"docs":{"steps":steps}}
    });
    let profiles = json!({"profiles":{"de":target("/home/alice/Dokumente", &["team"]), "fr":target("/home/alice/Documents", &[])}});
    fs::write(&path, profiles.to_string()).unwrap();
    check(temp.path(), &dir, Some(&path), true);
}

#[test]
fn profiles_select_architecture_specific_steps_using_dsc_platform_spellings() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("repo");
    let path = temp.path().join("profiles.yaml");
    let mut amd = package("amd");
    amd["condition"] = "[equals(context().os.architecture, 'x86_64')]".into();
    let mut arm = package("arm");
    arm["condition"] = "[or(equals(context()['os']['architecture'], 'aarch64'), equals(context().os.architecture, 'arm64'))]".into();
    app(&dir, "tool", json!({"resources":[amd, arm]}));
    let mut profiles = json!({});
    for (family, target, step, skip) in [
        ("Linux", "x86_64", "amd", "arm"), ("Windows", "aarch64", "arm", "amd"), ("macOS", "arm64", "arm", "amd"),
    ] {
        profiles[family] = json!({"family":family,"architecture":target,"packageManagers":[],"expect":{"tool":{"steps":[step],"skipped":[skip]}}});
    }
    fs::write(&path, json!({"profiles":profiles}).to_string()).unwrap();
    let report = check(temp.path(), &dir, Some(&path), true);
    assert!(report.contains("amd (depends on host)"), "{report}");
    fs::write(&path, "profiles: {linux: {family: Linux, packageManagers: []}}").unwrap();
    assert!(check(temp.path(), &dir, Some(&path), false).contains("set architecture in the target profile"));
}

#[test]
fn mise_profiles_select_native_mappings_first_and_accept_backend_tool_ids_on_all_platforms() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("repo");
    let path = temp.path().join("targets.yaml");
    let mut step = package("kargo");
    step["properties"]["namesByManager"] = json!({"brew":"kargo","mise":"github:akuity/kargo"});
    app(&dir, "kargo", json!({"resources":[step]}));
    let mut profiles = json!({});
    for (family, managers, manager, name) in [
        ("Linux", vec!["apt", "mise"], "mise", "github:akuity/kargo"),
        ("Windows", vec!["winget", "mise"], "mise", "github:akuity/kargo"),
        ("macOS", vec!["brew", "mise"], "brew", "kargo"),
    ] {
        profiles[family] = json!({"family":family, "packageManagers":managers, "expect":{"kargo":{"packages":{"kargo":{"manager":manager,"name":name}}}}});
    }
    fs::write(&path, json!({"profiles":profiles}).to_string()).unwrap();
    check(temp.path(), &dir, Some(&path), true);
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
        ("[contains(context().os, 'version')]", "only context().os.family and context().os.architecture are supported by the target profile"),
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
fn guarded_nix_steps_use_only_the_destinations_in_the_target_profile() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("repo");
    let path = temp.path().join("targets.yaml");
    let nix_step = |name: &str, extra: Value| {
        let mut props = json!({"name":name, "onlyIfSetUp":true});
        props.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        json!({"name":name, "type":"Furnish/NixOS", "properties":props})
    };
    app(&dir, "tool", json!({"resources":[
        nix_step("ssh-include", json!({"homeOptions":{"programs.ssh.includes":["config.d/extra.conf"]}})),
        nix_step("system-daemon", json!({"options":{"services.example.enable":true}})),
        nix_step("portable", json!({"packages":["hello"]})),
        {"name":"ordinary", "type":"Furnish/NixOS", "properties":{"name":"ordinary"}},
        {"name":"line-fallback", "type":"Furnish/Line", "condition":"[not(variables('furnish.host').nixSetup.home)]",
         "properties":{"path":"~/.ssh/config", "line":"Include config.d/extra.conf"}},
        {"name":"system-fact", "type":"Furnish/File", "condition":"[variables('furnish.host').nixSetup.system]",
         "properties":{"path":"/etc/example", "content":"system"}}
    ]}));
    let absent = "expect: {tool: {steps: [ordinary, line-fallback], skipped: [ssh-include, system-daemon, portable, system-fact]}}";
    let home = "expect: {tool: {steps: [ssh-include, portable, ordinary], skipped: [system-daemon, line-fallback, system-fact]}}";
    fs::write(&path, format!("profiles:
  debian: {{family: Linux, distribution: debian, packageManagers: [apt], {absent}}}
  mac: {{family: macOS, packageManagers: [brew, nix], {absent}}}
  windows: {{family: Windows, packageManagers: [winget], {absent}}}
  linux-home: {{family: Linux, packageManagers: [nix], nixSetup: {{home: true}}, {home}}}
  mac-home: {{family: macOS, packageManagers: [nix], nixSetup: {{home: true}}, {home}}}
  nixos:
    family: Linux
    packageManagers: [nix]
    nixSetup: {{system: true}}
    expect: {{tool: {{steps: [system-daemon, portable, ordinary, line-fallback, system-fact], skipped: [ssh-include]}}}}
  nixos-home:
    family: Linux
    packageManagers: [nix]
    nixSetup: {{system: true, home: true}}
    expect: {{tool: {{steps: [ssh-include, system-daemon, portable, ordinary, system-fact], skipped: [line-fallback]}}}}
")).unwrap();
    let output = check(temp.path(), &dir, Some(&path), true);
    for message in ["no NixOS or Home Manager configuration is set up in this profile", "Home Manager is not set up in this profile",
        "the NixOS configuration is not set up in this profile", "unavailable on Windows", "ssh-include (depends on host)"] {
        assert!(output.contains(message), "{output}");
    }

    // A skipped declaration cannot hide a later assertion failure.
    fs::write(&path, "profiles: {debian: {family: Linux, packageManagers: [apt], expect: {tool: {steps: [ssh-include]}}}}").unwrap();
    let output = check(temp.path(), &dir, Some(&path), false);
    assert!(output.contains("expected steps") && output.contains("line-fallback"), "{output}");
}

#[test]
fn a_declared_nix_setup_needs_a_supported_target_and_nix() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("repo");
    let path = temp.path().join("profiles.yaml");
    app(&dir, "tool", json!({"resources":[package("tool")]}));
    fs::write(&path, "profiles:
  windows: {family: Windows, packageManagers: [winget], nixSetup: {home: true}}
  mac: {family: macOS, packageManagers: [nix], nixSetup: {system: true}}
  without-nix: {family: Linux, packageManagers: [apt], nixSetup: {system: true}}
").unwrap();
    let output = check(temp.path(), &dir, Some(&path), false);
    for diagnostic in ["nixSetup.home requires a Linux or macOS profile", "nixSetup.system requires a Linux profile", "nixSetup requires nix in packageManagers"] {
        assert!(output.contains(diagnostic), "{output}");
    }
}

#[test]
fn family_preview_combines_system_guards_with_unknown_host_conditions() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("repo");
    let linux = "equals(context().os.family, 'Linux')";
    let mac = "equals(context()['os'].family, 'macOS')";
    let nix = "contains(variables('furnish.host').packageManagers, 'nix')";
    let brew = "contains(variables('furnish.host').packageManagers, 'brew')";
    let on_linux = format!("[and({linux}, {nix})]");
    let on_mac = format!("[and(not({brew}), {mac})]");
    let step = |name: &str, expr: &str| {
        let mut value = package(name);
        value["condition"] = expr.into();
        value
    };
    app(&dir, "dependency", json!({
        "resources":[package("library")],
        "metadata":{"furnish":{"dependencies":[{"app":"tools","condition": on_mac}]}}
    }));
    let mut doc = json!({
        "metadata":{"furnish":{"dependencies":[{"app":"dependency","condition": on_linux}]}},
        "resources":[step("kargo", &on_linux), step("homebrew", &on_mac),
            step("fallback", &format!("[or({nix}, {linux})]")),
            step("portable", &format!("[not(and({mac}, {nix}, true))]"))]
    });
    app(&dir, "tools", doc.clone());
    let report = check(temp.path(), &dir, None, true);
    let systems = "Windows: fallback (depends on host), portable · macOS: homebrew (depends on host), fallback (depends on host), portable (depends on host) · Linux: kargo (depends on host), fallback, portable";
    assert!(report.contains(systems), "{report}");
    assert!(!report.contains("form a cycle"), "{report}");

    // An impossible OS guard must still validate the rest of the condition.
    doc["resources"][0]["condition"] = json!("[and(false, equals(variables('furnish.host').misspelled, 'x'))]");
    app(&dir, "tools", doc);
    let report = check(temp.path(), &dir, None, false);
    assert!(report.contains("kargo: condition") && report.contains("misspelled"), "{report}");
}
