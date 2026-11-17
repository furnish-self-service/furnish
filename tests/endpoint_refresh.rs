//! Issue #1: the description of an endpoint the user added is fetched again when furnish updates
//! all collections, and a setting in it that isn't valid doesn't stop saving other settings. And
//! collections the endpoint lists later are added.
//! Runs the furnish binary against an endpoint on 127.0.0.1 (plain http is allowed there), which
//! serves its collections with git's "dumb" HTTP protocol (files of bare repositories).

use std::{
    fs,
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{Arc, Mutex},
};

use serde_json::{Value, json};

/// Serves `/.well-known/furnish.json` with whatever `doc` holds (`None`: HTTP 503), and
/// `/git/<name>.git/...` from the bare repository `<repos>/<name>.git` to requests with the token
/// `id` only.
fn serve(doc: Arc<Mutex<Option<String>>>, repos: Option<PathBuf>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(&stream);
            let mut line = String::new();
            let _ = reader.read_line(&mut line);
            let path = line.split(' ').nth(1).unwrap_or_default().split('?').next().unwrap_or_default().to_string();
            let mut signed_in = false;
            line.clear();
            while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                signed_in |= line.trim().eq_ignore_ascii_case("authorization: bearer id");
                line.clear();
            }
            let ok = |body: &[u8]| {
                let mut r = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
                r.extend_from_slice(body);
                r
            };
            let status = |s: &str| format!("HTTP/1.1 {s}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes();
            let response = if path == "/.well-known/furnish.json" {
                match doc.lock().unwrap().clone() {
                    Some(body) => ok(body.as_bytes()),
                    None => status("503 Service Unavailable"),
                }
            } else if let (Some(repos), Some(file)) = (&repos, path.strip_prefix("/git/")) {
                match fs::read(repos.join(file)) {
                    _ if !signed_in || file.contains("..") => status("401 Unauthorized"),
                    Ok(body) => ok(&body),
                    Err(_) => status("404 Not Found"),
                }
            } else {
                status("404 Not Found")
            };
            let _ = (&stream).write_all(&response);
        }
    });
    origin
}

struct Env {
    root: PathBuf,
    data: PathBuf,
    policy: PathBuf,
}

impl Env {
    fn furnish(&self, args: &[&str]) -> Output {
        let out = Command::new(env!("CARGO_BIN_EXE_furnish"))
            .args(args)
            .env("FURNISH_DATA_DIR", &self.data)
            .env("FURNISH_SYSTEM_DIR", self.root.join("system"))
            .env("FURNISH_POLICY_FILE", &self.policy)
            .env("FURNISH_TOKEN_STORE", "file")
            .env("HOME", &self.root)
            .output()
            .unwrap();
        eprintln!("furnish {}:\n{}{}", args.join(" "), String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        out
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn read(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn a_user_added_endpoint_is_fetched_again_and_a_bad_setting_spoils_nothing() {
    let root = std::env::temp_dir().join(format!("furnish-endpoint-refresh-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let env = Env { data: root.join("data"), policy: root.join("policy.json"), root: root.clone() };
    fs::create_dir_all(env.data.join("endpoints")).unwrap();
    fs::write(&env.policy, "{}").unwrap();

    let doc = Arc::new(Mutex::new(None));
    let origin = serve(doc.clone(), None);
    let description = |settings: Value, issuer: &str| {
        json!({
            "name": "Acme",
            "issuer": issuer,
            "clientId": "furnish",
            "collections": [{"name": "acme", "url": format!("{origin}/git/acme.git")}],
            "settings": settings
        })
    };
    let issuer = format!("{origin}/idp");
    let stem = origin.replace("://", "_").replace(':', "_");
    let saved = env.data.join("endpoints").join(format!("{stem}.json"));
    let token = env.data.join("endpoints").join(format!("{stem}.token"));

    // Added by the user (no admin policy names it) while it served the admin policy's wrapped
    // form (accepted now) and a value that isn't valid at all.
    let v1 = description(json!({"reporting": {"value": "errors"}, "updateIntervalMinutes": "often", "loadAwareUpdates": false}), &issuer);
    fs::write(&saved, json!({"origin": origin, "description": v1, "added": 42}).to_string()).unwrap();
    let tokens = json!({"issuer": issuer, "client_id": "furnish", "token_endpoint": format!("{issuer}/token"),
                        "id_token": "id", "access_token": "access", "refresh_token": "refresh", "expires_at": 4_000_000_000u64});
    fs::write(&token, tokens.to_string()).unwrap();

    // Saving any setting used to fail ("unknown variant `value`"): now the invalid value alone is
    // left out, and the endpoint's other settings apply.
    let out = env.furnish(&["settings", "--fallback-language", "de"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stderr(&out).contains("ignoring setting 'updateIntervalMinutes'"), "the ignored setting is named");
    assert!(stdout.contains("reports:           failures and fallbacks") && stdout.contains("when busy:         update anyway"));
    assert!(stdout.contains("every 30 minutes"));

    // The organization fixes its description; updating all collections fetches it again and keeps
    // when the endpoint was added and the sign-in.
    let v2 = description(json!({"reporting": "errors", "updateIntervalMinutes": 9}), &issuer);
    *doc.lock().unwrap() = Some(v2.to_string());
    assert!(env.furnish(&["collection", "update"]).status.success());
    let cached = read(&saved);
    assert_eq!(cached["description"]["settings"], v2["settings"]);
    assert_eq!(cached["added"], 42);
    assert_eq!(read(&token), tokens);
    let out = env.furnish(&["settings"]);
    assert!(!stderr(&out).contains("ignoring"), "no warnings left");
    assert!(String::from_utf8_lossy(&out.stdout).contains("every 9 minutes"));

    // A description that fails the checks: the saved one stays, and furnish says why.
    let mut bad = v2.clone();
    bad["collections"][0]["url"] = json!("https://elsewhere.tld/acme.git");
    *doc.lock().unwrap() = Some(bad.to_string());
    let out = env.furnish(&["collection", "update"]);
    assert!(stderr(&out).contains("isn't served by the endpoint itself") && stderr(&out).contains("keeping the saved description"));
    assert_eq!(read(&saved)["description"]["settings"], v2["settings"]);

    // The endpoint doesn't answer: the saved one stays, without a warning.
    *doc.lock().unwrap() = None;
    let out = env.furnish(&["collection", "update"]);
    assert!(out.status.success());
    assert!(!stderr(&out).contains("warning"));
    assert_eq!(read(&saved)["description"]["settings"], v2["settings"]);
    assert_eq!(read(&token), tokens);

    // Without usersCanAdd, endpoints the user added aren't fetched (nor used).
    fs::write(&env.policy, r#"{"collections": {"usersCanAdd": false}}"#).unwrap();
    let other = description(json!({}), &format!("{origin}/other"));
    *doc.lock().unwrap() = Some(other.to_string());
    env.furnish(&["collection", "update"]);
    assert_eq!(read(&saved)["description"]["settings"], v2["settings"]);
    fs::write(&env.policy, "{}").unwrap();

    // Another identity provider: the sign-in from the old one goes, people sign in again.
    assert!(env.furnish(&["collection", "update"]).status.success());
    assert_eq!(read(&saved)["description"]["issuer"], other["issuer"]);
    assert!(!token.exists());

    let _ = fs::remove_dir_all(&root);
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git").current_dir(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr));
}

/// A bare repository `<repos>/<name>.git` with one commit of these files, served as files.
fn repository(repos: &Path, name: &str, files: &[(&str, &str)]) -> PathBuf {
    let work = repos.join(format!("{name}.work"));
    fs::create_dir_all(&work).unwrap();
    git(&work, &["init", "--quiet", "-b", "main"]);
    for (file, content) in files {
        fs::write(work.join(file), content).unwrap();
    }
    git(&work, &["add", "-A"]);
    git(&work, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "--quiet", "-m", "init"]);
    let bare = repos.join(format!("{name}.git"));
    git(repos, &["clone", "--quiet", "--bare", &work.to_string_lossy(), &bare.to_string_lossy()]);
    git(&bare, &["update-server-info"]);
    bare
}

#[test]
fn collections_an_endpoint_lists_later_are_added_by_the_rules_for_adding() {
    let root = std::env::temp_dir().join(format!("furnish-endpoint-new-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let env = Env { data: root.join("data"), policy: root.join("policy.json"), root: root.clone() };
    let repos = root.join("repos");
    fs::create_dir_all(env.data.join("endpoints")).unwrap();
    fs::create_dir_all(&repos).unwrap();
    let plain = [("collection.yaml", "{}\n")];
    for name in ["acme", "tools", "mine", "it", "gone", "later"] {
        repository(&repos, name, &plain);
    }
    let question = "parameters:\n  role:\n    type: string\n    required: true\n    label: What do you work on?\n";
    repository(&repos, "asks", &[("collection.yaml", question)]);
    repository(&repos, "answered", &[("collection.yaml", question)]);
    // The organization presets a collection "it" from elsewhere; the person has their own "mine".
    let it = repository(&root, "it-preset", &plain);
    let policy = json!({"collections": {"preset": [{"name": "it", "url": it}]}});
    fs::write(&env.policy, policy.to_string()).unwrap();
    let mine = repository(&root, "mine-own", &plain);
    assert!(env.furnish(&["collection", "add", &mine.to_string_lossy(), "--name", "mine"]).status.success());

    let doc = Arc::new(Mutex::new(None));
    let origin = serve(doc.clone(), Some(repos.clone()));
    let issuer = format!("{origin}/idp");
    let description = |names: &[&str]| {
        let collections: Vec<_> = names.iter().map(|n| json!({"name": n, "url": format!("{origin}/git/{n}.git")})).collect();
        // Dumb HTTP has no partial clones; the address sets that for its own collections.
        json!({"name": "Acme", "issuer": issuer, "clientId": "furnish", "collections": collections,
               "settings": {"blobsOnDemand": false}, "parameters": {"answered": {"role": "design"}}})
    };
    let serving = |names: &[&str]| *doc.lock().unwrap() = Some(description(names).to_string());
    let stem = origin.replace("://", "_").replace(':', "_");
    let saved = env.data.join("endpoints").join(format!("{stem}.json"));
    let tokens = json!({"issuer": issuer, "client_id": "furnish", "token_endpoint": format!("{issuer}/token"),
                        "id_token": "id", "access_token": "access", "refresh_token": "refresh", "expires_at": 4_000_000_000u64});
    fs::write(env.data.join("endpoints").join(format!("{stem}.token")), tokens.to_string()).unwrap();
    let collections = env.data.join("collections");
    let update = || {
        let out = env.furnish(&["collection", "update"]);
        (String::from_utf8_lossy(&out.stdout).into_owned(), stderr(&out))
    };

    // Added with an earlier version of furnish, which didn't remember what it added: what the
    // address listed then counts as added. The person removed "acme" since.
    fs::write(&saved, json!({"origin": origin, "description": description(&["acme"]), "added": 42}).to_string()).unwrap();
    serving(&["acme", "tools", "asks", "answered", "mine", "it", "gone"]);
    let (stdout, stderr) = update();
    assert!(stdout.contains("Added 'tools'") && stdout.contains("Added 'asks'") && stdout.contains("Added 'answered'") && stdout.contains("Added 'gone'"));
    assert!(collections.join("tools/.git").exists() && collections.join("gone/.git").exists());
    assert!(!collections.join("acme").exists(), "a collection the person removed isn't added again");
    // Its questions are asked; the address answers those of its own collections.
    assert!(stderr.contains("asks asks: What do you work on?"), "the new collection's question is asked");
    assert!(!stderr.contains("answered asks"), "the address's answer counts");
    // Names that are taken: the organization's preset, the person's own collection.
    assert!(stderr.contains("not adding its new collection: 'it' is the name of a collection your organization provides"));
    assert!(stderr.contains("not adding its new collection: a collection named 'mine' already exists"));
    let list = String::from_utf8_lossy(&env.furnish(&["collection", "list"]).stdout).into_owned();
    assert!(list.lines().any(|l| l.contains("mine-own")), "the person's 'mine' stays theirs");
    assert!(list.lines().any(|l| l.starts_with("it ") && l.contains("managed")), "the preset stays the preset");

    // Removed on purpose: not added again, also after the address stops listing it for a while.
    assert!(env.furnish(&["collection", "remove", "tools"]).status.success());
    let (stdout, _) = update();
    assert!(!stdout.contains("Added") && !collections.join("tools").exists());
    serving(&["acme", "asks", "answered", "mine", "it"]);
    update();
    serving(&["acme", "tools", "asks", "answered", "mine", "it"]);
    update();
    assert!(!collections.join("tools").exists());

    // The address stopped listing "gone": the collection stays, with its apps.
    let list = String::from_utf8_lossy(&env.furnish(&["collection", "list"]).stdout).into_owned();
    assert!(list.lines().any(|l| l.starts_with("gone ")), "a collection the address stops listing stays");

    // Without usersCanAdd, nothing comes from addresses the person added; with it again, it does.
    serving(&["acme", "tools", "asks", "answered", "mine", "it", "later"]);
    fs::write(&env.policy, r#"{"collections": {"usersCanAdd": false}}"#).unwrap();
    update();
    assert!(!collections.join("later").exists());
    fs::write(&env.policy, policy.to_string()).unwrap();
    let (stdout, _) = update();
    assert!(stdout.contains("Added 'later'") && collections.join("later/.git").exists());
    assert_eq!(read(&saved)["added"], 42);

    let _ = fs::remove_dir_all(&root);
}
