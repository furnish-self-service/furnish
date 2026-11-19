//! Exercise the mise CLI contract through the same resource entry point as DSC, without network.
#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
    process::{Command, Stdio},
};

#[test]
fn mise_global_tool_lifecycle_and_shell_setup() {
    if unsafe { libc::geteuid() } == 0 {
        return;
    } // Furnish deliberately never runs mise as root.
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    let bin = home.join("host-bin");
    let shims = home.join("data/shims");
    fs::create_dir(&bin).unwrap();
    symlink(which::which("rm").unwrap(), bin.join("rm")).unwrap();
    fs::create_dir_all(&shims).unwrap();
    let profile = home.join(".bash_profile");
    fs::write(&profile, "# existing user configuration\n").unwrap();
    let tool = bin.join("mise");
    fs::write(
        &tool,
        r#"#!/bin/sh
set -eu
[ "$PWD" = / ] && [ "$MISE_YES" = 1 ] && [ -z "${MISE_NODE_VERSION:-}" ] || exit 91
case "$*" in
 'ls --global --json github:furnish/fixture')
    [ "$MISE_OFFLINE" = 1 ]
    if [ -e "$HOME/selected" ]; then
        read -r v < "$HOME/selected"
        if [ -e "$HOME/multi" ]; then
            printf '[{"version":"0.5.0","requested_version":"0.5","active":true,"installed":true},'
        else printf '['; fi
        printf '{"version":"%s","requested_version":"%s","active":true,"installed":true}]\n' "$v" "$v"
    else printf '[]\n'; fi;;
 'current github:furnish/fixture') read -r v < "$HOME/selected"; printf '%s 0.5.0\n' "$v";;
 'doctor --json')
    printf '{"dirs":{"data":"%s/data","shims":"%s/data/shims"},"activated":false,"shims_on_path":false}\n' "$HOME" "$HOME"
    exit 1;; # valid JSON even when an unrelated check warns
 'ls-remote github:furnish/fixture') printf '1.0.0\n2.0.0\n';;
 'latest github:furnish/fixture') printf '2.0.0\n';;
 'use --global github:furnish/fixture@1.0.0') echo 1.0.0 > "$HOME/selected";;
 'use --global github:furnish/fixture@latest github:furnish/fixture@0.5') [ -e "$HOME/multi" ]; echo 2.0.0 > "$HOME/selected";;
 'unuse --global github:furnish/fixture@2.0.0 github:furnish/fixture@0.5') rm "$HOME/selected" "$HOME/multi";;
 *) echo "unexpected arguments: $*" >&2; exit 92;;
esac
"#,
    )
    .unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
    // A shim left over from a local project is not an external installation, even through a PATH alias.
    let cmd = "fixture-program";
    fs::write(shims.join(cmd), "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(shims.join(cmd), fs::Permissions::from_mode(0o755)).unwrap();
    let alias = home.join("alias");
    symlink(&shims, &alias).unwrap();
    let path = std::env::join_paths([&bin, &alias]).unwrap();
    let props = json!({"name":"github:furnish/fixture","packageManager":"mise","command":cmd});
    let resource = |kind: &str, op: &str, request: Value, search_path: &std::ffi::OsStr, preview: bool, ok: bool| {
        let mut process = Command::new(env!("CARGO_BIN_EXE_furnish"));
        process.args(["dsc-resource", kind, op]);
        if preview {
            process.arg("--what-if");
        }
        let mut child = process
            .current_dir(home)
            .env("HOME", home)
            .env("PATH", search_path)
            .env("SHELL", "/bin/bash")
            .env("MISE_NODE_VERSION", "an-inherited-project-override")
            .env_remove("FURNISH_SERVICE")
            .env("FURNISH_POLICY_FILE", home.join("policy.json"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(request.to_string().as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(
            out.status.success(),
            ok,
            "{kind} {op} {request}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        if ok {
            serde_json::from_slice(&out.stdout).unwrap()
        } else {
            Value::String(String::from_utf8(out.stderr).unwrap())
        }
    };
    let run = |op: &str, extra: Value, preview: bool, ok: bool| {
        let mut request = props.clone();
        request.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        resource("package", op, request, &path, preview, ok)
    };
    let pin = json!({"targetVersion":"1.0.0"});
    let remove = json!({"_exist":false});
    let state = run("get", json!({"reportVersions":"all"}), false, true);
    assert_eq!(state["_exist"], false);
    assert_eq!(state["latestVersion"], "2.0.0");
    assert_eq!(state["availableVersions"], json!(["1.0.0", "2.0.0"]));
    assert_eq!(state["supportsVersionPin"], true);
    assert_eq!(run("test", pin.clone(), false, true)["_inDesiredState"], false);
    assert_eq!(run("set", pin.clone(), true, true)["_metadata"]["furnishNewSession"], true);
    assert!(!home.join("selected").exists() && !home.join(".bashrc").exists());
    // Externally managed shell configuration is left intact, including earlier files in the plan.
    fs::set_permissions(&profile, fs::Permissions::from_mode(0o444)).unwrap();
    let error = run("set", pin.clone(), false, false);
    assert!(error.as_str().unwrap().contains("read-only"));
    assert!(!home.join("selected").exists() && !home.join(".bashrc").exists());
    fs::set_permissions(&profile, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(run("set", pin.clone(), false, true)["version"], "1.0.0");
    // The dependent command runs in the same session, whose PATH still lacks the shims.
    let check_tool = json!({"test":{"command":[cmd],"exitCode":1},"set":[cmd]});
    let check = || resource("command", "test", check_tool.clone(), bin.as_os_str(), false, true);
    assert_eq!(check()["_inDesiredState"], true);
    // A program already on PATH retains precedence over the shim.
    let native = bin.join(cmd);
    fs::write(&native, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&native, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(check()["_inDesiredState"], false);
    fs::remove_file(native).unwrap();

    let written = fs::read_to_string(&profile).unwrap();
    assert!(written.starts_with("# existing user configuration\n"));
    assert_eq!(run("test", pin, false, true)["_inDesiredState"], true);
    fs::write(home.join("multi"), "").unwrap();
    assert_eq!(run("get", json!({}), false, true)["version"], "1.0.0");
    assert_eq!(run("test", json!({}), false, true)["_inDesiredState"], false);
    assert_eq!(run("set", json!({}), false, true)["version"], "2.0.0");
    assert_eq!(run("test", json!({}), false, true)["_inDesiredState"], true);
    assert_eq!(fs::read_to_string(&profile).unwrap(), written);
    assert_eq!(run("set", remove.clone(), false, true)["_exist"], false);
    assert_eq!(run("test", remove.clone(), false, true)["_inDesiredState"], true);

    fs::remove_file(&tool).unwrap();
    fs::remove_file(shims.join(cmd)).unwrap();
    let external = bin.join(cmd);
    fs::write(&external, "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(&external, fs::Permissions::from_mode(0o755)).unwrap();
    let state = run("get", json!({"reportVersions":"all"}), false, true);
    assert_eq!(state["managedBy"], "other");
    assert_eq!(state["path"].as_str().map(Path::new), Some(external.as_path()));
    assert!(state["latestVersion"].is_null());
    assert!(run("set", remove, false, false).as_str().unwrap().contains("outside furnish"));
}
