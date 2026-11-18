//! A desktop app that saves a cached document on exit can undo a live settings edit.
use serde_json::{Value, json};
use std::{
    env, fs,
    io::{self, BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

const FIXTURE_SETTINGS: &str = "FURNISH_FIXTURE_CACHED_SETTINGS";

fn read_settings(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

// Run this test executable under a distinct basename. The child loads the fixture before
// signalling readiness, waits for its input to close, then saves its cached copy on exit.
struct CachedApp(Child);

impl Drop for CachedApp {
    fn drop(&mut self) {
        drop(self.0.stdin.take());
        let status = self.0.wait().unwrap();
        if !std::thread::panicking() {
            assert!(status.success(), "cached settings app exited with {status}");
        }
    }
}

fn start_app(executable: &Path, settings: &Path) -> CachedApp {
    let mut child = Command::new(executable)
        .args(["--exact", "cached_settings_survive_relaunch", "--nocapture"])
        .env(FIXTURE_SETTINGS, settings)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let reader = BufReader::new(child.stdout.as_mut().unwrap());
    assert!(reader.lines().any(|line| line.unwrap().contains("fixture settings loaded")));
    CachedApp(child)
}

fn furnish(root: &Path, args: &[&str], input: Option<&Value>, success: bool) -> String {
    let mut command = Command::new(env!("CARGO_BIN_EXE_furnish"));
    command
        .args(args)
        .env("HOME", root)
        .env("USERPROFILE", root)
        .env("FURNISH_DATA_DIR", root.join("data"))
        .env("FURNISH_SYSTEM_DIR", root.join("system"))
        .env("FURNISH_POLICY_FILE", root.join("policy.json"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    if let Some(input) = input {
        serde_json::to_writer(child.stdin.take().unwrap(), input).unwrap();
    }
    let out = child.wait_with_output().unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert_eq!(out.status.success(), success, "{args:?}: {text}");
    text
}

fn app_file(repo: &Path, settings: &Path, guard: Option<Value>) {
    let mut props = json!({"path":settings,"location":"$.agentEnabled","value":true});
    let mut resources = Vec::new();
    if let Some(guard) = guard {
        props["requiresClosed"] = guard;
        resources.push(json!({"name":"Earlier setup","type":"Furnish/File",
            "properties":{"path":settings.with_extension("marker"),"content":"ready"}}));
    }
    resources.push(json!({"name":"Enable agent","type":"Furnish/Value","properties":props}));
    let app = json!({"resources":resources});
    fs::write(repo.join("apps/settings.dsc.yaml"), app.to_string()).unwrap();
}

fn fixture_executable(root: &Path) -> (String, PathBuf) {
    let name = format!(
        "settings-cache-{}",
        root.file_name().unwrap().to_string_lossy().trim_start_matches('.')
    );
    let path = root.join(format!("{name}{}", env::consts::EXE_SUFFIX));
    fs::copy(env::current_exe().unwrap(), &path).unwrap();
    (name, path)
}

fn assert_blocked(text: &str, process: &str) {
    let lower = text.to_lowercase();
    assert!(
        text.contains(process) && lower.contains("close") && lower.contains("retry"),
        "{text}"
    );
}

#[test]
fn cached_settings_survive_relaunch() {
    if let Some(path) = env::var_os(FIXTURE_SETTINGS) {
        let mut cached = read_settings(Path::new(&path));
        println!("fixture settings loaded");
        io::stdout().flush().unwrap();
        io::stdin().read_to_end(&mut Vec::new()).unwrap();
        cached["saves"] = json!(cached["saves"].as_u64().unwrap() + 1);
        fs::write(path, cached.to_string()).unwrap();
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let repo = root.join("data/collections/test");
    fs::create_dir_all(repo.join("apps")).unwrap();
    fs::write(root.join("policy.json"), "{}").unwrap();
    let settings = root.join("settings.json");
    fs::write(&settings, r#"{"agentEnabled":false,"saves":0}"#).unwrap();
    let (name, executable) = fixture_executable(root);
    app_file(&repo, &settings, None);
    for args in [&["init", "-q"][..], &["add", "apps"], &["commit", "-qm", "settings fixture"]] {
        assert!(
            Command::new("git")
                .current_dir(&repo)
                .args([
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.invalid",
                    "-c",
                    "commit.gpgsign=false"
                ])
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }

    // Reproduce the stale write under the original unguarded collection declaration.
    let app = start_app(&executable, &settings);
    furnish(root, &["install", "test/settings"], None, true);
    assert_eq!(read_settings(&settings)["agentEnabled"], true);
    drop(app);
    assert_eq!(read_settings(&settings), json!({"agentEnabled":false,"saves":1}));

    app_file(&repo, &settings, Some(json!(["absent-fixture-process", name])));
    let app = start_app(&executable, &settings);
    let preview = furnish(root, &["install", "--dry-run", "test/settings"], None, true);
    assert!(preview.to_lowercase().contains("close") && preview.contains(&name), "{preview}");
    let expected = fs::read(&settings).unwrap();
    let blocked = furnish(root, &["install", "test/settings"], None, false);
    assert_blocked(&blocked, &name);
    assert_eq!(fs::read(&settings).unwrap(), expected);
    assert!(!settings.with_extension("marker").exists(), "the operation must pause before its first write");
    drop(app);

    furnish(root, &["install", "test/settings"], None, true);
    let app = start_app(&executable, &settings);
    let noop = furnish(root, &["install", "--dry-run", "test/settings"], None, true);
    assert!(!noop.to_lowercase().contains("close"), "{noop}");
    furnish(root, &["install", "test/settings"], None, true);
    let blocked = furnish(root, &["uninstall", "test/settings"], None, false);
    assert_blocked(&blocked, &name);
    drop(app);
    assert_eq!(read_settings(&settings), json!({"agentEnabled":true,"saves":3}));
    furnish(root, &["uninstall", "test/settings"], None, true);
    assert_eq!(read_settings(&settings), json!({"saves":3}));

    app_file(&repo, &settings, Some(json!(["../invalid"])));
    let validation = furnish(root, &["check", repo.to_str().unwrap()], None, false);
    assert!(validation.contains("requiresClosed"), "{validation}");
}

#[test]
fn file_resources_guard_each_mutation_and_allow_noops() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let (name, executable) = fixture_executable(root);
    let settings = root.join("fixture.json");
    fs::write(&settings, r#"{"saves":0}"#).unwrap();
    let cases = [
        ("file", json!({"content":"managed"})),
        ("line", json!({"line":"managed"})),
        ("block", json!({"content":"managed","app":"test/block"})),
        ("value", json!({"location":"$.managed","value":true})),
    ];
    let resource = |kind: &str, op: &str, props: &Value, success| furnish(root, &["dsc-resource", kind, op], Some(props), success);
    let app = start_app(&executable, &settings);
    for (kind, mut props) in cases {
        let path = root.join(format!("{kind}.json"));
        props["path"] = json!(path);
        props["requiresClosed"] = json!([name]);
        let blocked = resource(kind, "set", &props, false);
        assert_blocked(&blocked, &name);
        assert!(!path.exists());
        let mut unguarded = props.clone();
        unguarded.as_object_mut().unwrap().remove("requiresClosed");
        resource(kind, "set", &unguarded, true);
        let expected = fs::read(&path).unwrap();
        resource(kind, "set", &props, true);
        assert_eq!(fs::read(&path).unwrap(), expected);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
            props["mode"] = json!("600");
            assert_blocked(&resource(kind, "set", &props, false), &name);
            assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o644);
        }
        props["_exist"] = json!(false);
        unguarded["_exist"] = json!(false);
        let preview = furnish(root, &["dsc-resource", kind, "set", "--what-if"], Some(&props), true);
        assert!(preview.to_lowercase().contains("close") && preview.contains(&name), "{preview}");
        assert_blocked(&resource(kind, "set", &props, false), &name);
        assert_eq!(fs::read(&path).unwrap(), expected);
        resource(kind, "set", &unguarded, true);
        resource(kind, "set", &props, true);
    }
    drop(app);
}

#[cfg(target_os = "linux")]
#[test]
fn shared_electron_blocks_only_the_running_package() {
    // A shell copied under a shared runtime's basename gives the kernel real /proc
    // entries with the same executable and argument layout as the packaged launchers.
    // Each fixture's first operand is its package; the script stays alive until EOF.
    fn launch(exe: &Path, archive: &Path, later_argument: &Path) -> CachedApp {
        fs::create_dir_all(archive.parent().unwrap()).unwrap();
        fs::write(archive, "echo ready\nread -r line\nexit 0\n").unwrap();
        let mut child = Command::new(exe).arg(archive).arg(later_argument)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        let mut ready = String::new();
        BufReader::new(child.stdout.as_mut().unwrap()).read_line(&mut ready).unwrap();
        assert_eq!(ready, "ready\n");
        CachedApp(child)
    }

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let product = format!("Notes-{}", root.file_name().unwrap().to_str().unwrap());
    let wanted = product.to_ascii_lowercase();
    let native = root.join("electron");
    let versioned = root.join("electron43");
    let decoy = root.join("unrelated-runtime");
    for exe in [&native, &versioned, &decoy] {
        fs::copy("/bin/sh", exe).unwrap();
    }
    let nix = root.join("opt").join(&product).join("resources/app.asar");
    let arch = root.join("lib").join(&wanted).join("app.asar");
    let other = root.join("lib/AnotherApp/app.asar");
    let _other = launch(&native, &other, &nix);
    let _decoy = launch(&decoy, &arch, &nix);
    let settings = root.join("settings.json");
    let props = json!({"path":settings,"location":"$.enabled","value":true,"requiresClosed":[wanted]});
    let set = |success| furnish(root, &["dsc-resource", "value", "set"], Some(&props), success);

    for (exe, target) in [(&native, &nix), (&versioned, &arch)] {
        fs::write(&settings, "{}").unwrap();
        let process = launch(exe, target, &other);
        assert_blocked(&set(false), &wanted);
        assert_eq!(fs::read(&settings).unwrap(), b"{}");
        drop(process);
        set(true);
        assert_eq!(read_settings(&settings)["enabled"], true);
    }
}
