//! A managed collection supplies its own package backend. Checking a collection and
//! loading it under another name must never grant access to that backend.
use serde_json::json;
use std::{fs, path::Path, process::Command};

fn git(dir: &Path, args: &[&str]) {
    let result = Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
}

#[test]
fn only_the_owning_managed_collection_runs_its_script() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let repo = root.join("repository");
    let policy = root.join("policy.json");
    fs::create_dir_all(repo.join("apps")).unwrap();
    fs::create_dir_all(repo.join("managers")).unwrap();
    #[cfg(unix)]
    let (file, required, script) = (
        "company.sh",
        "sh",
        r#"set -eu
IFS= read -r input
case "$input" in *'"name":"company-tool"'*) ;; *) echo "unexpected input: $input" >&2; exit 1;; esac
case "$input" in *'"_exist":false'*) want=false;; *) want=true;; esac
printf '%s\n' "$1" >> "$PROBE_DIR/calls"
have=false; [ ! -e "$PROBE_DIR/installed" ] || have=true
case "$1" in
    get) ;;
    test)
        ok=false; [ "$want" != "$have" ] || ok=true
        printf '{"name":"company-tool","_exist":%s,"_inDesiredState":%s}\n' "$have" "$ok"; exit 0 ;;
    what-if) have=$want ;;
    set)
        if [ "$want" = true ]; then touch "$PROBE_DIR/installed"; else rm -f "$PROBE_DIR/installed"; fi
        have=$want ;;
    *) exit 1 ;;
esac
printf '{"name":"company-tool","_exist":%s}\n' "$have"
"#,
    );
    #[cfg(windows)]
    let (file, required, script) = (
        "company.ps1",
        "powershell.exe",
        r#"param([string]$Operation)
$ErrorActionPreference = 'Stop'
$request = [Console]::In.ReadToEnd() | ConvertFrom-Json
if ($request.name -cne 'company-tool') { throw "Unexpected package name: $($request.name)" }
[IO.File]::AppendAllText((Join-Path $env:PROBE_DIR 'calls'), "$Operation`n")
$marker = Join-Path $env:PROBE_DIR 'installed'
$state = @{ name = $request.name; _exist = [IO.File]::Exists($marker) }
switch ($Operation) {
    'get' { }
    'test' { $state._inDesiredState = $request._exist -eq $state._exist }
    'what-if' { $state._exist = $request._exist }
    'set' {
        if ($request._exist) { [IO.File]::WriteAllText($marker, '') }
        else { [IO.File]::Delete($marker) }
        $state._exist = [IO.File]::Exists($marker)
    }
    default { throw "Unexpected operation: $Operation" }
}
[Console]::WriteLine(($state | ConvertTo-Json -Compress))
"#,
    );
    fs::write(
        repo.join("collection.yaml"),
        format!("packageManagers:\n  company: {{ script: managers/{file}, requiresCommand: {required} }}\n"),
    )
    .unwrap();
    fs::write(repo.join("managers").join(file), script).unwrap();
    let app = |guard: bool| {
        let mut resource = json!({"type":"Furnish/Package","name":"Tools","properties":{
            "name":"fallback","packageManager":"company","namesByManager":{"company":"company-tool"}}});
        if guard {
            resource["condition"] = "[contains(variables('furnish.host').packageManagers, 'company')]".into();
        }
        json!({"metadata":{"furnish":{"description":"Test","categories":["Tools"]}},"resources":[resource]}).to_string()
    };
    fs::write(repo.join("apps/tools.dsc.yaml"), app(true)).unwrap();
    fs::write(repo.join("apps/explicit.dsc.yaml"), app(false)).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "fixture"]);
    // Git for Windows stores local remotes with forward slashes. Policy compares the exact URL.
    let url = repo.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
    fs::write(
        &policy,
        json!({"collections":{"preset":[{"name":"managed", "url":url}]}}).to_string(),
    )
    .unwrap();
    let run = |args: &[&str], ok: bool| {
        let out = Command::new(env!("CARGO_BIN_EXE_furnish"))
            .args(args)
            .env("FURNISH_POLICY_FILE", &policy)
            .env("PROBE_DIR", root)
            .env("HOME", root)
            .env("FURNISH_DATA_DIR", root.join("test data"))
            .env("FURNISH_SYSTEM_DIR", root.join("system"))
            .output()
            .unwrap();
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert_eq!(out.status.success(), ok, "{args:?}: {text}");
        text
    };
    let calls = root.join("calls");
    run(&["check", &url], true);
    assert!(!calls.exists());
    let plan = run(&["install", "managed/tools", "--dry-run"], true);
    assert!(plan.contains("The app selects the collection’s company script manager"), "{plan}");
    assert!(plan.contains("Package name company-tool comes from the mapping for company"), "{plan}");
    let preview = fs::read_to_string(&calls).unwrap();
    assert!(preview.lines().any(|op| op == "what-if"), "{preview}");
    assert!(!preview.lines().any(|op| op == "set"));
    assert!(!root.join("installed").exists());
    run(&["install", "managed/tools"], true);
    assert!(root.join("installed").exists());
    let sets = || fs::read_to_string(&calls).unwrap().lines().filter(|&op| op == "set").count();
    let count = sets();
    run(&["install", "managed/tools"], true);
    assert_eq!(sets(), count);
    run(&["uninstall", "managed/tools"], true);
    assert!(!root.join("installed").exists());

    run(&["collection", "add", &url, "--name", "personal"], true);
    let count = sets();
    run(&["install", "personal/tools"], false);
    let missing = run(&["install", "personal/explicit"], false);
    assert!(missing.contains("needs") && missing.contains("company"), "{missing}");
    assert_eq!(sets(), count);
    assert!(!root.join("installed").exists());
}

#[test]
fn checker_reports_invalid_declarations_without_running_scripts() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir(root.join("apps")).unwrap();
    #[cfg(unix)]
    let escape = tempfile::NamedTempFile::new().unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(escape.path(), root.join("outside.sh")).unwrap();
    fs::write(
        root.join("apps/tool.dsc.yaml"),
        json!({"resources":[{"name":"tool","type":"Furnish/Package",
        "properties":{"name":"tool","packageManager":"company"}}]})
        .to_string(),
    )
    .unwrap();
    for (yaml, message) in [
        ("packageManagers: { company: { script: ../secret.sh } }", "inside the collection"),
        #[cfg(unix)]
        ("packageManagers: { company: { script: outside.sh } }", "inside the collection"),
        ("packageManagers: { company: { script: missing.sh } }", "is missing"),
        ("packageManagers: { apt: { script: missing.sh } }", "built-in package manager"),
        ("packageManagers: {}", "not built in or declared"),
    ] {
        fs::write(root.join("collection.yaml"), yaml).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_furnish"))
            .arg("check")
            .arg(root)
            .env("HOME", root)
            .env("FURNISH_DATA_DIR", root.join("data"))
            .env("FURNISH_SYSTEM_DIR", root.join("system"))
            .env("FURNISH_POLICY_FILE", root.join("empty-policy.json"))
            .output()
            .unwrap();
        assert!(!out.status.success(), "{yaml}");
        let output = String::from_utf8_lossy(&out.stdout);
        assert!(output.contains(message), "{yaml}: {output}");
    }
}
