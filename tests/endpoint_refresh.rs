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
    let discovery = json!({"issuer":origin,"authorization_endpoint":format!("{origin}/authorize"),"token_endpoint":format!("{origin}/token")}).to_string();
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
            } else if path == "/.well-known/openid-configuration" {
                ok(discovery.as_bytes())
            } else if path.starts_with("/limited.git") {
                status("429 Too Many Requests")
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
    assert!(stdout.lines().any(|line| line == "reports:           errors"));
    assert!(stdout.contains("when busy:         update anyway"));
    let reports = env.furnish(&["reports"]);
    assert!(reports.status.success());
    assert_eq!(String::from_utf8_lossy(&reports.stdout).lines().next(), Some("Reports: errors"));
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

    // Invalid or newer selectors cannot replace a working release track with the default branch.
    let mut tagged = v2.clone();
    tagged["collections"][0]["tracking"] = json!({"mode":"tag", "pattern":"^v[0-9]+$"});
    *doc.lock().unwrap() = Some(tagged.to_string());
    let out = env.furnish(&["collection", "update"]);
    assert!(out.status.success(), "{}", stderr(&out));
    for invalid in [json!({"mode":"channel", "name":"next"}), json!({"mode":"tag", "pattern":"["}), Value::Null] {
        let mut bad = tagged.clone();
        bad["collections"][0]["tracking"] = invalid;
        *doc.lock().unwrap() = Some(bad.to_string());
        let out = env.furnish(&["collection", "update"]);
        assert!(stderr(&out).contains("keeping the saved description"), "{}", stderr(&out));
        assert_eq!(read(&saved)["description"]["collections"], tagged["collections"]);
    }

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
    let mut cmd = Command::new("git");
    // Not a repository that `git rebase -x` or a hook points at (see catalog::GIT_LOCATION_VARS).
    for var in furnish::catalog::GIT_LOCATION_VARS {
        cmd.env_remove(var);
    }
    let out = cmd.current_dir(dir).args(args).output().unwrap();
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

#[cfg(unix)]
#[test]
fn service_updates_from_its_own_fresh_or_last_valid_endpoint_description() {
    use std::{io::Read, os::unix::net::UnixStream, process::{Child, Stdio}, time::{Duration, Instant}};
    struct Service(Child);
    impl Drop for Service {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    let repos = root.join("repos");
    fs::create_dir(&repos).unwrap();
    let bare = repository(&repos, "tools", &[("channel", "main")]);
    let work = repos.join("tools.work");
    git(&work, &["checkout", "-qb", "stable"]);
    fs::write(work.join("channel"), "stable").unwrap();
    git(&work, &["add", "channel"]);
    git(&work, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "stable"]);
    git(&work, &["push", "-q", &bare.to_string_lossy(), "stable"]);
    git(&bare, &["update-server-info"]);
    let doc = Arc::new(Mutex::new(None));
    let origin = serve(doc.clone(), Some(repos));
    let description = |branch: &str| json!({
        "name":"Test","clientId":"test","issuer":origin,"settings":{"blobsOnDemand":false},
        "collections":[{"name":"tools","url":format!("{origin}/git/tools.git"),"tracking":{"mode":"branch","name":branch}}]
    });
    let system = root.join("system");
    let endpoints = system.join("endpoints");
    fs::create_dir_all(&endpoints).unwrap();
    let stem = origin.replace("://", "_").replace(':', "_");
    fs::write(endpoints.join(format!("{stem}.token")), json!({
        "issuer":origin,"client_id":"test","token_endpoint":format!("{origin}/token"),
        "id_token":"id","access_token":"access","refresh_token":"refresh","expires_at":4_000_000_000u64
    }).to_string()).unwrap();
    let policy = root.join("policy.json");
    fs::write(&policy, json!({"service":{},"endpoints":[origin]}).to_string()).unwrap();
    *doc.lock().unwrap() = Some(description("main").to_string());
    let socket = root.join("service.sock");
    let mut service = Service(Command::new(env!("CARGO_BIN_EXE_furnish")).arg("service")
        .env("FURNISH_SOCKET",&socket).env("FURNISH_SYSTEM_DIR",&system).env("FURNISH_POLICY_FILE",policy)
        .env("FURNISH_TOKEN_STORE","file").env("HOME",root).stdout(Stdio::null()).spawn().unwrap());
    let until = Instant::now() + Duration::from_secs(30);
    while !socket.exists() {
        assert!(service.0.try_wait().unwrap().is_none() && Instant::now() < until, "service failed to start");
        std::thread::sleep(Duration::from_millis(50));
    }
    let channel = system.join("collections/tools/channel");
    assert_eq!(fs::read_to_string(&channel).unwrap(), "main");
    // The caller's sign-in sets up the shared preset; a personal copy would be invisible
    // in the catalog and could follow a different source from the service's checkout.
    let personal = root.join("personal");
    fs::create_dir_all(personal.join("endpoints")).unwrap();
    fs::copy(endpoints.join(format!("{stem}.token")), personal.join("endpoints").join(format!("{stem}.token"))).unwrap();
    fs::remove_dir_all(system.join("collections/tools")).unwrap();
    let add = Command::new(env!("CARGO_BIN_EXE_furnish")).args(["collection", "add", &origin])
        .env("FURNISH_DATA_DIR", &personal).env("FURNISH_SYSTEM_DIR", &system).env("FURNISH_POLICY_FILE", root.join("policy.json"))
        .env("FURNISH_SOCKET", &socket).env("FURNISH_TOKEN_STORE", "file").env("HOME", root).output().unwrap();
    assert!(add.status.success(), "{}", stderr(&add));
    assert_eq!(fs::read_to_string(&channel).unwrap(), "main");
    assert!(!personal.join("collections/tools").exists());
    // A client can request an update by name but cannot supply a branch or description.
    let update = || {
        let mut stream = UnixStream::connect(&socket).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        writeln!(stream, "{}", json!({"update":{"collection":"tools","headers":{origin.clone(): "Authorization: Bearer id"}}})).unwrap();
        let mut replies = String::new();
        stream.read_to_string(&mut replies).unwrap();
        assert!(replies.lines().any(|line| serde_json::from_str::<Value>(line).unwrap()["updated"].is_object()), "{replies}");
    };
    *doc.lock().unwrap() = Some(description("stable").to_string());
    update();
    assert_eq!(fs::read_to_string(&channel).unwrap(), "stable");
    let saved = endpoints.join(format!("{stem}.json"));
    assert_eq!(read(&saved)["description"]["collections"], description("stable")["collections"]);
    // The description is unavailable but Git still serves the last valid source.
    *doc.lock().unwrap() = None;
    update();
    assert_eq!(fs::read_to_string(channel).unwrap(), "stable");
}

#[test]
fn failed_collection_updates_leave_a_private_local_log_with_reporting_off() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let env = Env { root: root.into(), data: root.join("data"), policy: root.join("policy.json") };
    fs::write(&env.policy, "{}").unwrap();
    let repo = repository(root, "tools", &[("collection.yaml", "{}\n")]);
    let log = env.data.join("furnish.log");
    assert!(env.furnish(&["collection", "add", &repo.to_string_lossy(), "--name", "tools"]).status.success());
    assert!(env.furnish(&["collection", "update", "tools"]).status.success());
    assert!(!log.exists());

    let origin = serve(Arc::new(Mutex::new(None)), None);
    git(&env.data.join("collections/tools"), &["remote", "set-url", "origin", &format!("{origin}/limited.git?token=private-value")]);
    let update = env.furnish(&["collection", "update", "tools"]);
    assert!(!update.status.success());
    let entry = read(&log);
    assert!(entry["time"].as_str().unwrap().ends_with('Z'));
    assert_eq!(entry["collection"], "tools");
    assert_eq!(entry["event"], "collectionUpdate");
    assert_eq!(entry["outcome"], "failed");
    assert!(entry["error"].as_str().unwrap().contains("429"));
    assert!(!entry.to_string().contains("private-value"));
    assert!(fs::read(env.data.join("reports/queue.ndjson")).unwrap_or_default().is_empty());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&log).unwrap().permissions().mode() & 0o777, 0o600);
    }
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
    // The endpoint's release branch and its default branch have different content.
    let tools_work = repos.join("tools.work");
    git(&tools_work, &["checkout", "-qb", "stable"]);
    fs::write(tools_work.join("release-channel"), "stable").unwrap();
    git(&tools_work, &["add", "release-channel"]);
    git(&tools_work, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "release"]);
    git(&tools_work, &["push", "-q", &repos.join("tools.git").to_string_lossy(), "stable"]);
    git(&repos.join("tools.git"), &["update-server-info"]);
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
        let collections: Vec<_> = names.iter().map(|n| {
            let mut c = json!({"name": n, "url": format!("{origin}/git/{n}.git")});
            if *n == "tools" { c["tracking"] = json!({"mode":"branch","name":"stable"}); }
            c
        }).collect();
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

    assert_eq!(fs::read_to_string(collections.join("tools/release-channel")).unwrap(), "stable");
    // The same endpoint can change the policy default after its initial offer.
    let mut changed = description(&["acme", "tools", "asks", "answered", "mine", "it"]);
    changed["collections"][1]["tracking"] = json!({"mode": "default"});
    *doc.lock().unwrap() = Some(changed.to_string());
    update();
    assert!(!collections.join("tools/release-channel").exists());

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

#[test]
fn newest_tag_on_authenticated_dumb_http_uses_the_published_release() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let repos = root.join("repos");
    let env = Env { data: root.join("data"), policy: root.join("policy.json"), root: root.to_owned() };
    let endpoints = env.data.join("endpoints");
    fs::write(&env.policy, "{}").unwrap();
    fs::create_dir_all(&endpoints).unwrap();
    let bare = repository(&repos, "tools", &[("collection.yaml", "{}\n"), ("channel", "release")]);
    let work = repos.join("tools.work");
    git(&work, &["-c", "user.name=t", "-c", "user.email=t@t", "-c", "tag.gpgSign=false", "tag", "-a", "v1", "-m", "release"]);
    fs::write(work.join("channel"), "development").unwrap();
    git(&work, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qam", "development"]);
    git(&work, &["push", "-q", "--tags", &bare.to_string_lossy(), "main"]);
    git(&bare, &["update-server-info"]);

    let doc = Arc::new(Mutex::new(None));
    let origin = serve(doc.clone(), Some(repos));
    let url = format!("{origin}/git/tools.git");
    let issuer = format!("{origin}/idp");
    let description = json!({"name":"Acme","issuer":issuer,"clientId":"furnish","settings":{"blobsOnDemand":false},
        "collections":[{"name":"tools","url":url,"tracking":{"mode":"tag","pattern":"^v[0-9]+$","order":"newest"}}]});
    let stem = origin.replace("://", "_").replace(':', "_");
    fs::write(endpoints.join(format!("{stem}.json")), json!({"origin":origin,"description":description,"added":42}).to_string()).unwrap();
    fs::write(endpoints.join(format!("{stem}.token")), json!({"issuer":issuer,"client_id":"furnish","token_endpoint":format!("{issuer}/token"),
        "id_token":"id","access_token":"access","expires_at":4_000_000_000u64}).to_string()).unwrap();
    *doc.lock().unwrap() = Some(description.to_string());

    let out = env.furnish(&["collection", "add", &url, "--name", "tools"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(fs::read_to_string(env.data.join("collections/tools/channel")).unwrap(), "release");
    let out = env.furnish(&["collection", "update", "tools"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(fs::read_to_string(env.data.join("collections/tools/channel")).unwrap(), "release");
}
