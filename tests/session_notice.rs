//! The CLI reports a login requirement from the real preview and completed DSC operation.
#![cfg(unix)]

use serde_json::{Value, json};
use std::{fs, process::Command};

#[test]
fn preview_and_success_report_profile_changes_and_ignore_noops_and_failed_installs() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let repo = root.join("data/collections/test");
    fs::create_dir_all(repo.join("apps")).unwrap();
    let file = |path: &str| json!({"name":"write","type":"Furnish/File","properties":{"path":path,"content":"export EXAMPLE=1\n"}});
    let save = |id: &str, doc: Value| fs::write(repo.join("apps").join(format!("{id}.dsc.yaml")), doc.to_string()).unwrap();
    save("profile", json!({"resources":[file("~/.profile")]}));
    save(
        "declared",
        json!({"metadata":{"furnish":{"requiresNewSession":true}}, "resources":[file("~/declared")]}),
    );
    let mut permissions = file("~/.profile");
    permissions["properties"]["mode"] = json!("600");
    save("permissions", json!({"resources":[permissions]}));
    let mut skipped = file("~/.zprofile");
    skipped["condition"] = json!("[equals(1, 2)]");
    save("ordinary", json!({"resources":[file("~/ordinary"),skipped]}));
    save(
        "failed",
        json!({"resources":[file("~/.bash_login"),{
            "type":"Furnish/Command", "name":"fail", "dependsOn":["[resourceId('Furnish/File','write')]"],
            "properties":{"test":{"command":["/bin/false"]},"set":["/bin/false"]}
        }]}),
    );
    for args in [vec!["init", "-q"], vec!["add", "apps"], vec!["commit", "-qm", "collection"]] {
        assert!(
            Command::new("git")
                .current_dir(&repo)
                .args([
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=t@example.test",
                    "-c",
                    "commit.gpgsign=false"
                ])
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
    let run = |args: &[&str], ok: bool, notice: bool| {
        let out = Command::new(env!("CARGO_BIN_EXE_furnish"))
            .args(args)
            .env("HOME", root)
            .env("FURNISH_DATA_DIR", root.join("data"))
            .env("FURNISH_SYSTEM_DIR", root.join("system"))
            .env("FURNISH_POLICY_FILE", root.join("policy.json"))
            .output()
            .unwrap();
        let output = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert_eq!(out.status.success(), ok, "{args:?}: {output}");
        assert_eq!(
            output.to_ascii_lowercase().contains("sign out of your computer"),
            notice,
            "{args:?}: {output}"
        );
    };
    for app in ["test/profile", "test/declared"] {
        run(&["install", "--dry-run", app], true, true);
        run(&["install", app], true, true);
        run(&["install", "--dry-run", app], true, false);
        run(&["install", app], true, false);
    }
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(root.join(".profile"), fs::Permissions::from_mode(0o644)).unwrap();
    run(&["install", "--dry-run", "test/permissions"], true, false);
    run(&["install", "test/permissions"], true, false);
    assert_eq!(fs::metadata(root.join(".profile")).unwrap().permissions().mode() & 0o777, 0o600);
    run(&["install", "--dry-run", "test/ordinary"], true, false);
    run(&["install", "test/ordinary"], true, false);
    run(&["install", "test/failed"], false, false);
    assert!(
        root.join(".bash_login").is_file(),
        "the failing command must have followed the successful profile change"
    );
    run(&["uninstall", "--dry-run", "test/profile"], true, false);
}
