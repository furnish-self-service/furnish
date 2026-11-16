//! Issue #1: the description of an endpoint the user added is fetched again when furnish updates
//! all collections, and a setting in it that isn't valid doesn't stop saving other settings.
//! Runs the furnish binary against an endpoint on 127.0.0.1 (plain http is allowed there).

use std::{
    fs,
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{Arc, Mutex},
};

use serde_json::{Value, json};

/// Serves `/.well-known/furnish.json` with whatever `doc` holds; `None`: HTTP 503.
fn serve(doc: Arc<Mutex<Option<String>>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(&stream);
            let mut line = String::new();
            while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                line.clear();
            }
            let response = match doc.lock().unwrap().clone() {
                Some(body) => format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()),
                None => "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
            };
            let _ = (&stream).write_all(response.as_bytes());
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
    let origin = serve(doc.clone());
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
