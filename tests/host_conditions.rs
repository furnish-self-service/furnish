//! Host facts travel through the app's metadata, DSC defaults and the actual DSC resource.
use serde_json::json;
use std::{fs, process::Command};

#[test]
fn host_facts_gate_dependencies_steps_and_resource_properties() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let repo = root.join("data/collections/test");
    fs::create_dir_all(repo.join("apps")).unwrap();
    let host = "variables('furnish.host')";
    let present = format!("[contains({host}, 'packageManagers')]");
    let install = |name: &str, doc: serde_json::Value| {
        fs::write(repo.join("apps").join(format!("{name}.dsc.yaml")), doc.to_string()).unwrap();
    };
    let file = |name: &str, condition: &str, content: &str| {
        json!({"name":name,"type":"Furnish/File","condition":condition,
            "properties":{"path":root.join(name),"content":content}})
    };
    install("dependency", json!({"resources":[file("from-dependency", &present, "dependency")]}));
    install(
        "app",
        json!({
            "metadata":{"furnish":{"dependencies":[{"app":"dependency","required":true,"condition":"[parameters('enable')]"}]}},
            "parameters":{"enable":{"type":"bool","defaultValue": present}},
            "resources":[file("excluded", &format!("[contains({host}.packageManagers, 'a-manager-that-does-not-exist')]"), "skipped"),
                file("from-app", "[parameters('enable')]", &format!("[concat('distro=', {host}.distribution)]"))]
        }),
    );
    let documents = |name: &str| install(name, json!({
        "metadata":{"furnish":{"perUser":true}},
        "parameters":{"location":{"type":"string","defaultValue":format!("[concat({host}.documentsDir, '/Team docs')]")}},
        "resources":[file(name, &present, "[parameters('location')]")]
    }));
    documents("documents");
    documents("other-documents");
    let git = |args: &[&str]| {
        assert!(
            Command::new("git")
                .args(["-c", "user.name=Test", "-c", "user.email=test@example.invalid", "-c", "commit.gpgsign=false"])
                .args(args)
                .current_dir(&repo)
                .status()
                .unwrap()
                .success()
        );
    };
    git(&["init", "-q"]);
    git(&["add", "apps"]);
    git(&["commit", "-qm", "fixture"]);
    let furnish = |args: &[&str], ok: bool| {
        let out = Command::new(env!("CARGO_BIN_EXE_furnish"))
            .args(args)
            .env("FURNISH_DATA_DIR", root.join("data"))
            .env("FURNISH_SYSTEM_DIR", root.join("system"))
            .env("FURNISH_POLICY_FILE", root.join("policy.json"))
            .env("HOME", root)
            .env("XDG_CONFIG_HOME", root.join("xdg"))
            .output()
            .unwrap();
        let output = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert_eq!(out.status.success(), ok, "{args:?}: {output}");
        output
    };
    let output = furnish(&["install", "test/app"], true);
    assert!(output.contains("[1/1] from-app"), "{output}");
    assert!(output.contains("excluded skipped"), "{output}");
    assert_eq!(fs::read_to_string(root.join("from-dependency")).unwrap(), "dependency");
    assert!(fs::read_to_string(root.join("from-app")).unwrap().starts_with("distro="));
    assert!(!root.join("excluded").exists());

    // Keep writes in the fixture even when Windows redirects the actual known folder to OneDrive.
    #[cfg(target_os = "linux")]
    let expected = {
        furnish(&["install", "test/documents"], true);
        assert_eq!(fs::read_to_string(root.join("documents")).unwrap(), format!("{}/Team docs", root.join("Documents").display()));
        fs::create_dir_all(root.join("xdg")).unwrap();
        fs::write(root.join("xdg/user-dirs.dirs"), "XDG_DOCUMENTS_DIR=\"$HOME/Mes documents\"\n").unwrap();
        root.join("Mes documents")
    };
    #[cfg(target_os = "macos")]
    let expected = root.join("Documents");
    #[cfg(windows)]
    let expected = dirs::document_dir().expect("Documents known folder");
    furnish(&["install", "test/other-documents"], true);
    assert_eq!(fs::read_to_string(root.join("other-documents")).unwrap(), format!("{}/Team docs", expected.display()));
    #[cfg(target_os = "linux")]
    furnish(&["uninstall", "test/documents"], true);
    furnish(&["option", "set", "test/documents", "location", "My override"], true);
    furnish(&["install", "test/documents"], true);
    assert_eq!(fs::read_to_string(root.join("documents")).unwrap(), "My override");

    install("fedora", json!({"resources":[file("fedora-only", &format!("[equals({host}.distribution, 'fedora')]"), "fedora")]}));
    let path = repo.to_str().unwrap();
    let report = furnish(&["check", path], true);
    assert!(report.contains("fedora-only (depends on host)"), "{report}");
    install("collision", json!({"variables":{"furnish.host":{}},"resources":[]}));
    let report = furnish(&["check", path], false);
    assert!(report.contains("cannot be redeclared"), "{report}");
}
