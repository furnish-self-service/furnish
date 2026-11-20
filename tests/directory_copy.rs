//! The installed check, preview, update and uninstall all use the same owned files.
use serde_json::json;
use std::{fs, process::Command};

#[test]
fn synchronize_collection_tree_and_preserve_local_files_on_update_and_uninstall() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let repo = root.join("data/collections/team");
    let source = repo.join("docs/offline");
    let target = root.join("Team docs");
    fs::create_dir_all(repo.join("apps")).unwrap();
    fs::create_dir_all(source.join("manual")).unwrap();
    fs::write(source.join("manual/old"), "old").unwrap();
    fs::write(source.join("README"), "first").unwrap();
    let app = json!({"resources":[{"name":"offline docs","type":"Furnish/Directory",
        "properties":{"path":target,"source":"docs/offline/"}}]});
    fs::write(repo.join("apps/docs.dsc.yaml"), app.to_string()).unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .current_dir(&repo)
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.org",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "first"]);
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_furnish"))
            .args(args)
            .env("FURNISH_DATA_DIR", root.join("data"))
            .env("FURNISH_SYSTEM_DIR", root.join("system"))
            .env("FURNISH_POLICY_FILE", root.join("policy.json"))
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.status.success(), "{args:?}: {text}");
        text
    };
    assert!(run(&["install", "--dry-run", "docs"]).contains("added: 2, changed: 0, removed: 0"));
    assert!(!target.exists());
    run(&["install", "docs"]);
    assert!(run(&["list"]).contains("✓  docs"));
    fs::write(target.join("manual/personal-notes"), "mine").unwrap();
    fs::remove_file(source.join("manual/old")).unwrap();
    fs::write(source.join("README"), "second").unwrap();
    fs::write(source.join("new.rpm"), [0, 1, 254]).unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-qm", "docs update"]);
    // Installer-shaped file names are excluded by Furnish's sparse checkout.
    git(&["sparse-checkout", "set", "--no-cone", "/*", "!*.rpm"]);
    assert!(!source.join("new.rpm").exists());
    assert!(run(&["list"]).contains("↑  docs"));
    let preview = run(&["install", "--dry-run", "docs"]);
    assert!(preview.contains("added: 1, changed: 1, removed: 1"), "{preview}");
    assert!(target.join("manual/old").exists());
    run(&["install", "docs"]);
    assert_eq!(fs::read(target.join("new.rpm")).unwrap(), [0, 1, 254]);
    assert_eq!(fs::read(target.join("README")).unwrap(), b"second");
    assert!(!target.join("manual/old").exists());
    assert!(run(&["list"]).contains("✓  docs"));
    run(&["uninstall", "docs"]);
    assert!(!target.join("README").exists());
    assert!(!target.join("new.rpm").exists());
    assert_eq!(fs::read(target.join("manual/personal-notes")).unwrap(), b"mine");
    assert!(!run(&["list"]).contains("✓  docs"));
}
