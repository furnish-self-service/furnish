//! Exercise the DSC installer resource, native ownership, and version-policy handling through the CLI.
use serde_json::json;
use std::{fs, process::Command};

#[test]
fn an_external_program_satisfies_a_pinned_installer_offline() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let repo = root.join("collection");
    fs::create_dir_all(repo.join("apps")).unwrap();
    let tools = root.join("tools");
    fs::create_dir(&tools).unwrap();
    let program = "furnish-external-fixture";
    #[cfg(unix)]
    let (properties, probe) = {
        use std::{os::unix::fs::PermissionsExt, path::Path};
        fn shell(path: &Path, body: &str) {
            fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let dir = root.join(".local/bin");
        fs::create_dir_all(&dir).unwrap();
        let probe = dir.join(program);
        shell(&probe, "touch \"$HOME/executed\"");
        shell(&tools.join("installer"), "touch \"$HOME/executed\"");
        shell(&tools.join("pkgutil"), "test -e \"$HOME/receipt\" && printf 'version: 2.1\\n'");
        (
            json!({"url":"https://127.0.0.1:9/Homebrew.pkg","packageId":"sh.fixture.unique"}),
            probe,
        )
    };
    #[cfg(windows)]
    let (properties, probe) = {
        let dir = root.join("npm");
        fs::create_dir(&dir).unwrap();
        let probe = dir.join(format!("{program}.cmd"));
        fs::write(&probe, "@echo executed > \"%USERPROFILE%\\executed\"").unwrap();
        (
            json!({"url":"https://127.0.0.1:9/tool.exe","displayName":"Furnish External Fixture","installArgs":"/S"}),
            probe,
        )
    };
    let mut properties = properties;
    properties["command"] = program.into();
    properties["sha256"] = "a".repeat(64).into();
    properties["versions"] = json!([">=2"]);
    fs::write(
        repo.join("apps/tool.dsc.yaml"),
        json!({
            "metadata":{"furnish":{"description":"Fixture","categories":["Test"]}},
            "resources":[{"name":"Install tool","type":"Furnish/Installer","properties":properties}]
        })
        .to_string(),
    )
    .unwrap();
    for args in [
        &["init", "--quiet", "-b", "main"][..],
        &["add", "."],
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "fixture",
        ],
    ] {
        assert!(Command::new("git").current_dir(&repo).args(args).status().unwrap().success());
    }
    let mut paths = vec![tools];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let path = std::env::join_paths(paths).unwrap();
    let run = |args: &[&str], success| {
        let output = Command::new(env!("CARGO_BIN_EXE_furnish"))
            .args(args)
            .env("HOME", root)
            .env("APPDATA", root)
            .env("USERPROFILE", root)
            .env("PATH", &path)
            .env("FURNISH_DATA_DIR", root.join("data"))
            .env("FURNISH_SYSTEM_DIR", root.join("system"))
            .env("FURNISH_POLICY_FILE", root.join("policy.json"))
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.status.success(), success, "{args:?}: {text}");
        text
    };
    fs::write(root.join("policy.json"), "{}").unwrap();
    run(&["collection", "add", repo.to_str().unwrap(), "--name", "fixture"], true);
    let status = run(&["list"], true);
    assert!(
        status.contains("installed outside furnish") && status.contains(probe.to_str().unwrap()) && status.contains("version unknown"),
        "{status}"
    );
    run(&["install", "fixture/tool", "--dry-run"], true);
    run(&["install", "fixture/tool"], true);
    let denied = run(&["uninstall", "fixture/tool"], false);
    assert!(denied.contains("installed outside furnish") && denied.contains(program), "{denied}");
    assert!(!root.join("executed").exists());
    assert!(probe.is_file());

    #[cfg(unix)]
    {
        fs::write(root.join("receipt"), "").unwrap();
        let status = run(&["list"], true);
        assert!(
            !status.contains("installed outside furnish") && !status.contains("version unknown"),
            "{status}"
        );
        let plan = run(&["uninstall", "fixture/tool", "--dry-run"], true);
        assert!(plan.contains("pkgutil --forget sh.fixture.unique"), "{plan}");
    }
}
