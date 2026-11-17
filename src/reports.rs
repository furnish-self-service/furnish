//! Reports to the organization: what furnish installed, updated or removed on this computer and
//! whether it worked, sent as OpenTelemetry logs (OTLP/HTTP with JSON) to the `events` URL of the
//! organization's furnish address (see `endpoint`). Its proxy names the signed-in user to the
//! admins' OTLP receiver, which adds it to each record; furnish never sends a user name itself.
//!
//! The setting `reporting` decides what is recorded: `off` (default), `errors` (failures,
//! fallbacks and required versions that aren't met) or `all` (also successes and a daily
//! inventory). Events wait in
//! `reports/queue.ndjson` in the data directory until an endpoint accepts them, so offline
//! computers report later. Users can read that file; `furnish reports` shows it. Each endpoint gets
//! only the reports about its own collections (see `policy::speaking_for_own`).

use std::{
    cell::Cell,
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// What the setting `reporting` allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Level {
    Off,
    Errors,
    All,
}

/// One report. `record` adds when, from where, and with which furnish.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    /// install, update, uninstall, reinstall, fallback, collectionUpdate, inventory or requirement.
    pub event: &'static str,
    /// succeeded, failed or cancelled; `reported` for inventory; notAcceptable, noneOffered or
    /// acceptable for requirement (see [`requirement`]).
    pub outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collection: Option<String>,
    /// The collection's commit the app was installed from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// user, update (automatic updates), autoInstall (the organization's first-start apps).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Requirement: the installed version of the app.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed_version: Option<String>,
    /// Requirement: the newest version the package manager offers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offered_version: Option<String>,
    /// Inventory: every app of the catalog that is installed, with its status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apps: Option<Vec<Installed>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Installed {
    pub app: String,
    /// installed or needsUpdate.
    pub status: &'static str,
}

/// The event and the context `record` adds to every report.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report<'a> {
    time: String,
    device: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    host: Option<String>,
    os: String,
    os_version: String,
    furnish: &'static str,
    #[serde(flatten)]
    event: &'a Event,
}

static IN_SERVICE: AtomicBool = AtomicBool::new(false);

/// The furnish service records nothing: the client that asked it reports the outcome, signed in.
pub fn running_as_service() {
    IN_SERVICE.store(true, Ordering::Relaxed);
}

thread_local! {
    static TRIGGER: Cell<&'static str> = const { Cell::new("user") };
}

/// Run `f` with events recorded as caused by `trigger` (update, autoInstall) instead of the user.
pub fn with_trigger<T>(trigger: &'static str, f: impl FnOnce() -> T) -> T {
    let previous = TRIGGER.replace(trigger);
    let result = f();
    TRIGGER.set(previous);
    result
}

pub fn version() -> &'static str {
    crate::selfupdate::current()
}

fn dir() -> PathBuf {
    crate::catalog::data_dir().join("reports")
}

pub fn queue_path() -> PathBuf {
    dir().join("queue.ndjson")
}

/// The queue keeps at most this much; older reports go first.
const MAX_QUEUE: u64 = 5 << 20;
/// At most this much log records per request (the endpoint's proxies accept 1 MiB), leaving room
/// for the resources around them.
const MAX_BATCH: usize = 900 << 10;

pub fn level() -> Level {
    crate::settings::load().reporting
}

/// Times an operation and records it with its outcome.
pub fn operation<T>(event: &'static str, app: &crate::catalog::App, version: Option<String>, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let started = Instant::now();
    let result = f();
    let outcome = match &result {
        Ok(_) => "succeeded",
        Err(err) if err.is::<crate::engine::Cancelled>() => "cancelled",
        Err(_) => "failed",
    };
    record(Event {
        event,
        outcome,
        app: Some(app.key()),
        collection: Some(app.collection.clone()),
        version,
        trigger: Some(TRIGGER.get()),
        error: result.as_ref().err().map(|e| format!("{e:#}")),
        duration_ms: Some(started.elapsed().as_millis() as u64),
        ..Default::default()
    });
    result
}

/// Queue an event if the setting `reporting` asks for it.
pub fn record(event: Event) {
    queue(event);
}

/// Queue an event if the setting `reporting` asks for it; whether it was queued.
fn queue(event: Event) -> bool {
    let important = event.outcome == "failed" || event.event == "fallback" || event.event == "requirement";
    let wanted = match level() {
        Level::Off => false,
        Level::Errors => important,
        Level::All => true,
    };
    if !wanted || IN_SERVICE.load(Ordering::Relaxed) {
        return false;
    }
    match append(&event) {
        Ok(()) => true,
        Err(err) => {
            eprintln!("warning: couldn't queue a report: {err:#}");
            false
        }
    }
}

/// An app's unmet requirement as last recorded (kept in `reports/requirements.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Unmet {
    /// notAcceptable or noneOffered.
    outcome: String,
    installed: Option<String>,
    offered: Option<String>,
}

impl Unmet {
    /// What a status check found: `None` if the installed version is acceptable, or nothing is
    /// installed (installing fails then, and that is reported).
    fn of(required: Option<&crate::required::Required>) -> Option<Unmet> {
        let r = required?;
        if r.installed.is_none() || r.acceptable {
            return None;
        }
        let outcome = if r.target.is_none() { "noneOffered" } else { "notAcceptable" };
        Some(Unmet { outcome: outcome.into(), installed: r.installed.clone(), offered: r.offered.clone() })
    }
}

/// Record when an app's installed version stops or starts being acceptable (see `required`).
pub fn requirement(app: &crate::catalog::App, required: Option<&crate::required::Required>) {
    if IN_SERVICE.load(Ordering::Relaxed) {
        return;
    }
    requirement_in(&dir().join("requirements.json"), &app.key(), &app.collection, Unmet::of(required), queue);
}

/// Only changes are recorded: an unmet requirement (again with another outcome or installed
/// version; a newer offered version alone isn't a change), and `acceptable` after one. The state file changes only when the event was queued,
/// so a change with `reporting: off` is recorded once reporting is on.
fn requirement_in(state: &Path, app: &str, collection: &str, now: Option<Unmet>, queue: impl FnOnce(Event) -> bool) {
    let mut last: BTreeMap<String, Unmet> = fs::read(state).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let same = |a: &Unmet, b: &Unmet| (&a.outcome, &a.installed) == (&b.outcome, &b.installed);
    match (last.get(app), &now) {
        (None, None) => return,
        (Some(a), Some(b)) if same(a, b) => return,
        _ => {}
    }
    let event = Event {
        event: "requirement",
        outcome: match &now {
            Some(u) if u.outcome == "noneOffered" => "noneOffered",
            Some(_) => "notAcceptable",
            None => "acceptable",
        },
        app: Some(app.into()),
        collection: Some(collection.into()),
        installed_version: now.as_ref().and_then(|u| u.installed.clone()),
        offered_version: now.as_ref().and_then(|u| u.offered.clone()),
        ..Default::default()
    };
    if !queue(event) {
        return;
    }
    match now {
        Some(u) => last.insert(app.into(), u),
        None => last.remove(app),
    };
    let written = state.parent().map_or(Ok(()), fs::create_dir_all).and_then(|()| {
        crate::auth::write_private(state, serde_json::to_string(&last).unwrap_or_default().as_bytes())
    });
    if let Err(err) = written {
        eprintln!("warning: couldn't keep the reported versions in {}: {err}", state.display());
    }
}

fn append(event: &Event) -> Result<()> {
    let settings = crate::settings::load();
    let report = Report {
        time: rfc3339(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)),
        device: device_id()?,
        host: settings.report_hostname.then(sysinfo::System::host_name).flatten(),
        os: std::env::consts::OS.into(),
        os_version: sysinfo::System::os_version().unwrap_or_default(),
        furnish: version(),
        event: &Event { error: event.error.as_deref().map(redact), ..event.clone() },
    };
    private_dir()?;
    let path = queue_path();
    let mut file = crate::auth::open_private(&path, true)?;
    writeln!(file, "{}", serde_json::to_string(&report)?)?;
    if file.metadata()?.len() > MAX_QUEUE {
        let text = fs::read_to_string(&path)?;
        let keep = &text[text.len() / 2..];
        let keep = keep.find('\n').map_or("", |i| &keep[i + 1..]);
        crate::auth::write_private(&path, keep.as_bytes())?;
    }
    Ok(())
}

/// The reports directory, which only the user can open (on Unix).
fn private_dir() -> Result<()> {
    fs::create_dir_all(dir())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir(), fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// An error message without secrets: credentials in URLs (`https://user:token@host`, as git
/// prints them), `Bearer`/`Basic` credentials and anything that looks like a JWT become `***`.
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        let lower = rest.to_ascii_lowercase();
        // Userinfo after a scheme: up to the last '@' before the host ends.
        if let Some(after) = rest.strip_prefix("://") {
            let end = after.find(|c: char| c == '/' || c.is_whitespace() || "'\"<>".contains(c)).unwrap_or(after.len());
            out.push_str("://");
            match after[..end].rfind('@') {
                Some(at) => {
                    out.push_str("***");
                    out.push_str(&after[at..end]);
                }
                None => out.push_str(&after[..end]),
            }
            rest = &after[end..];
            continue;
        }
        let scheme = ["bearer ", "basic "].into_iter().find(|s| lower.starts_with(s));
        if let Some(scheme) = scheme {
            let after = &rest[scheme.len()..];
            let end = after.find(|c: char| !(c.is_ascii_alphanumeric() || "-._~+/=".contains(c))).unwrap_or(after.len());
            out.push_str(&rest[..scheme.len()]);
            out.push_str(if end > 0 { "***" } else { "" });
            rest = &after[end..];
            continue;
        }
        if rest.starts_with("eyJ") && (out.is_empty() || !out.ends_with(|c: char| c.is_ascii_alphanumeric())) {
            let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || "-_.".contains(c))).unwrap_or(rest.len());
            if rest[..end].matches('.').count() >= 2 {
                out.push_str("***");
                rest = &rest[end..];
                continue;
            }
        }
        let c = rest.chars().next().expect("not empty");
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// A random ID for this computer and user, kept in the data directory. Not derived from hardware.
fn device_id() -> Result<String> {
    let path = dir().join("device-id");
    if let Ok(id) = fs::read_to_string(&path) {
        return Ok(id.trim().to_string());
    }
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("no random numbers: {e}"))?;
    let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    private_dir()?;
    crate::auth::write_private(&path, id.as_bytes())?;
    Ok(id)
}

/// Where reports go: the `events` URL of each endpoint whose defaults apply, if it is on the
/// endpoint itself (the user's token only ever goes there), and the collections it gets reports
/// about.
fn destinations() -> Vec<(String, crate::endpoint::Description, String)> {
    crate::policy::endpoints_in_effect()
        .into_iter()
        .filter_map(|(origin, d)| {
            let url = d.events.clone()?;
            url.starts_with(&format!("{origin}/")).then_some((origin, d, url))
        })
        .collect()
}

/// Whether reports have somewhere to go.
pub fn destination_names() -> Vec<String> {
    destinations().into_iter().map(|(_, d, _)| d.name).collect()
}

/// Send queued reports. Reports the endpoints didn't accept stay queued for the next time.
pub fn send() {
    static SENDING: Mutex<()> = Mutex::new(());
    let Ok(_one) = SENDING.try_lock() else { return };
    if IN_SERVICE.load(Ordering::Relaxed) || waiting() {
        return;
    }
    // One sender across processes too (the desktop app and the CLI), so no batch goes twice.
    let Some(_lock) = Lock::take(dir().join("sending")) else { return };
    let targets = destinations();
    if targets.is_empty() {
        return;
    }
    loop {
        let Ok(text) = fs::read_to_string(queue_path()) else { return };
        let batch = first_batch(&text);
        if batch.is_empty() {
            return;
        }
        for (origin, description, url) in &targets {
            let own: Vec<&str> = description.collections.iter().map(|c| c.name.as_str()).collect();
            let theirs = about(batch, &own);
            if theirs.is_empty() {
                continue;
            }
            let Ok(token) = crate::endpoint::token(origin, description) else { return };
            match post(url, &token, &otlp(&theirs)) {
                Answer::Accepted => {}
                Answer::Rejected(why) => eprintln!("warning: {} refused {} reports, dropping them: {why}", description.name, theirs.lines().count()),
                Answer::Offline(why) => {
                    eprintln!("warning: couldn't send reports to {}: {why}", description.name);
                    return;
                }
                Answer::Later(why, retry_after) => {
                    eprintln!("warning: couldn't send reports to {}: {why}", description.name);
                    back_off(retry_after);
                    return;
                }
            }
        }
        let _ = fs::remove_file(dir().join("retry-at"));
        // Drop what was sent (reports about no endpoint's collections go nowhere); reports queued
        // meanwhile stay.
        let Ok(now) = fs::read_to_string(queue_path()) else { return };
        if let Some(rest) = now.strip_prefix(batch)
            && crate::auth::write_private(&queue_path(), rest.as_bytes()).is_err()
        {
            return;
        }
    }
}

/// What an endpoint answered, following OTLP/HTTP: 2xx accepted (rejected records in a partial
/// success are only logged), 429, 502, 503 and 504 retry later (after Retry-After), 400 and 413
/// drop the batch, since sending it again would fail again and hold up the queue.
enum Answer {
    Accepted,
    Rejected(String),
    /// Retry later, with the server's Retry-After in seconds if it sent one.
    Later(String, Option<u64>),
    /// No answer, or a sign-in problem: try again next time.
    Offline(String),
}

fn post(url: &str, token: &str, body: &str) -> Answer {
    let sent = crate::endpoint::http()
        .post(url)
        .config()
        .http_status_as_error(false)
        .build()
        .header("Authorization", &format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .send(body);
    let mut response = match sent {
        Ok(response) => response,
        Err(err) => return Answer::Offline(err.to_string()),
    };
    let status = response.status().as_u16();
    let retry_after = response.headers().get("Retry-After").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse().ok());
    let text = response.body_mut().with_config().limit(64 << 10).read_to_string().unwrap_or_default();
    let message = || {
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        let message = parsed["message"].as_str().map_or_else(|| text.trim().chars().take(200).collect(), str::to_string);
        format!("HTTP {status} {message}").trim().to_string()
    };
    match status {
        200..=299 => {
            let parsed: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
            let partial = &parsed["partialSuccess"];
            let rejected = partial["rejectedLogRecords"].as_str().and_then(|n| n.parse().ok()).or(partial["rejectedLogRecords"].as_u64()).unwrap_or(0);
            if rejected > 0 || partial["errorMessage"].as_str().is_some_and(|m| !m.is_empty()) {
                eprintln!("warning: the endpoint dropped {rejected} reports: {}", partial["errorMessage"].as_str().unwrap_or_default());
            }
            Answer::Accepted
        }
        429 | 502 | 503 | 504 => Answer::Later(message(), retry_after),
        400 | 413 => Answer::Rejected(message()),
        401 | 403 => Answer::Offline(message()),
        _ => Answer::Later(message(), None),
    }
}

/// After an answer to retry later: no sending before the server's Retry-After, or a delay that
/// doubles from 1 minute up to 1 hour with each failed try.
fn back_off(retry_after: Option<u64>) {
    let previous = fs::read_to_string(dir().join("retry-at")).ok().and_then(|t| t.split_whitespace().nth(1)?.parse::<u64>().ok());
    let delay = retry_after.unwrap_or_else(|| previous.map_or(60, |d| d * 2)).clamp(1, 3600);
    let at = unix_now() + delay;
    let _ = crate::auth::write_private(&dir().join("retry-at"), format!("{at} {delay}").as_bytes());
}

/// Whether an endpoint asked to wait before the next try.
fn waiting() -> bool {
    let at = fs::read_to_string(dir().join("retry-at")).ok().and_then(|t| t.split_whitespace().next()?.parse::<u64>().ok());
    at.is_some_and(|at| unix_now() < at)
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Report fields and the log record attributes they become.
const ATTRIBUTES: &[(&str, &str)] = &[
    ("event", "furnish.event"),
    ("outcome", "furnish.outcome"),
    ("app", "furnish.app"),
    ("collection", "furnish.collection"),
    ("version", "furnish.collection.commit"),
    ("trigger", "furnish.trigger"),
    ("error", "error.message"),
    ("durationMs", "furnish.duration_ms"),
    ("installedVersion", "furnish.installed_version"),
    ("offeredVersion", "furnish.offered_version"),
    ("apps", "furnish.apps"),
];

/// Queued reports as an OTLP ExportLogsServiceRequest in JSON: one resource per device and
/// furnish version, one log record per report.
fn otlp(lines: &str) -> String {
    let mut resources: Vec<(serde_json::Value, Vec<serde_json::Value>)> = Vec::new();
    for line in lines.lines() {
        let Ok(report) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let resource = otlp_resource(&report);
        let record = otlp_record(&report);
        match resources.iter_mut().find(|(r, _)| *r == resource) {
            Some((_, records)) => records.push(record),
            None => resources.push((resource, vec![record])),
        }
    }
    let resource_logs: Vec<_> = resources
        .into_iter()
        .map(|(attributes, records)| {
            serde_json::json!({
                "resource": {"attributes": attributes},
                "scopeLogs": [{"scope": {"name": "furnish"}, "logRecords": records}],
            })
        })
        .collect();
    serde_json::json!({"resourceLogs": resource_logs}).to_string()
}

/// Who sent a report, with OpenTelemetry's semantic conventions.
fn otlp_resource(report: &serde_json::Value) -> serde_json::Value {
    let os = match report["os"].as_str().unwrap_or_default() {
        "macos" => "darwin",
        os => os,
    };
    let mut attributes = vec![attribute("service.name", &"furnish".into()), attribute("service.version", &report["furnish"])];
    attributes.push(attribute("os.type", &os.into()));
    attributes.push(attribute("os.version", &report["osVersion"]));
    attributes.push(attribute("device.id", &report["device"]));
    if report["host"].is_string() {
        attributes.push(attribute("host.name", &report["host"]));
    }
    attributes.into()
}

fn otlp_record(report: &serde_json::Value) -> serde_json::Value {
    let text = |key: &str| report[key].as_str().unwrap_or_default();
    let (number, severity) = match (text("event"), text("outcome")) {
        (_, "failed") | ("requirement", "noneOffered") => (17, "ERROR"),
        ("fallback", _) | ("requirement", "notAcceptable") => (13, "WARN"),
        _ => (9, "INFO"),
    };
    let body = match report["apps"].as_array() {
        Some(apps) => format!("{} of {} app{}", text("event"), apps.len(), if apps.len() == 1 { "" } else { "s" }),
        None => {
            let subject = if report["app"].is_string() { text("app") } else { text("collection") };
            format!("{} {subject} {}", text("event"), text("outcome")).split_whitespace().collect::<Vec<_>>().join(" ")
        }
    };
    let attributes: Vec<_> = ATTRIBUTES.iter().filter(|(field, _)| !report[*field].is_null()).map(|(field, key)| attribute(key, &report[*field])).collect();
    serde_json::json!({
        "timeUnixNano": (parse_rfc3339(text("time")).unwrap_or_else(|| unix_now() * 1000) * 1_000_000).to_string(),
        "severityNumber": number,
        "severityText": severity,
        "body": {"stringValue": body},
        "attributes": attributes,
    })
}

fn attribute(key: &str, value: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({"key": key, "value": any_value(value)})
}

/// A JSON value as an OTLP AnyValue (64-bit integers as strings, as OTLP's JSON encoding wants).
fn any_value(value: &serde_json::Value) -> serde_json::Value {
    use serde_json::{Value, json};
    match value {
        Value::Bool(b) => json!({"boolValue": b}),
        Value::Number(n) if n.is_i64() || n.is_u64() => json!({"intValue": n.to_string()}),
        Value::Number(n) => json!({"doubleValue": n}),
        Value::String(s) => json!({"stringValue": s}),
        Value::Array(items) => json!({"arrayValue": {"values": items.iter().map(any_value).collect::<Vec<_>>()}}),
        Value::Object(map) => json!({"kvlistValue": {"values": map.iter().map(|(k, v)| attribute(k, v)).collect::<Vec<_>>()}}),
        Value::Null => json!({}),
    }
}

/// The reports of a batch about these collections: events about one of them, and inventories
/// with only their apps.
fn about(batch: &str, collections: &[&str]) -> String {
    let ours = |app: &str| collections.contains(&app.split('/').next().unwrap_or_default());
    let mut out = String::new();
    for line in batch.lines() {
        let Ok(mut report) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        if let Some(apps) = report.get_mut("apps").and_then(|a| a.as_array_mut()) {
            apps.retain(|a| a["app"].as_str().is_some_and(ours));
            if apps.is_empty() {
                continue;
            }
            out.push_str(&report.to_string());
        } else if report["collection"].as_str().is_some_and(|c| collections.contains(&c)) {
            out.push_str(line);
        } else {
            continue;
        }
        out.push('\n');
    }
    out
}

/// A lock file; a lock older than 10 minutes was left by a process that ended without cleaning up.
struct Lock(PathBuf);

impl Lock {
    fn take(path: PathBuf) -> Option<Lock> {
        let stale = fs::metadata(&path)
            .and_then(|m| m.modified())
            .is_ok_and(|t| t.elapsed().is_ok_and(|age| age.as_secs() > 600));
        if stale {
            let _ = fs::remove_file(&path);
        }
        fs::create_dir_all(path.parent()?).ok()?;
        fs::OpenOptions::new().write(true).create_new(true).open(&path).ok()?;
        Some(Lock(path))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Whole lines from the start of the queue whose log records take at most MAX_BATCH bytes (one
/// line at least).
fn first_batch(text: &str) -> &str {
    let (mut end, mut size) = (0, 0);
    for (i, _) in text.match_indices('\n') {
        let line = &text[end..i];
        size += serde_json::from_str::<serde_json::Value>(line).map_or(line.len(), |r| otlp_record(&r).to_string().len() + 1);
        if size > MAX_BATCH && end > 0 {
            break;
        }
        end = i + 1;
    }
    &text[..end]
}

/// With `reporting: all`: whether a day passed since the last inventory.
pub fn inventory_due() -> bool {
    if level() != Level::All {
        return false;
    }
    let last = fs::read_to_string(dir().join("inventory-at")).ok().and_then(|t| t.trim().parse::<u64>().ok()).unwrap_or(0);
    unix_now() > last + 86_400
}

/// Record which apps are installed (with `reporting: all`, once a day).
pub fn inventory(apps: Vec<Installed>) {
    record(Event { event: "inventory", outcome: "reported", apps: Some(apps), ..Default::default() });
    let now = unix_now();
    let _ = private_dir().and_then(|()| Ok(crate::auth::write_private(&dir().join("inventory-at"), now.to_string().as_bytes())?));
}

/// Milliseconds since 1970 as `2026-11-15T08:03:06.042Z`. Milliseconds keep two reports of the
/// same second apart (Loki keeps only one of two equal entries with the same time).
pub(crate) fn rfc3339(millis: u64) -> String {
    let secs = millis / 1000;
    // Days to civil date (Howard Hinnant's algorithm), valid from 1970 on.
    let days = (secs / 86_400) as i64 + 719_468;
    let era = days / 146_097;
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let s = secs % 86_400;
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z", s / 3600, s / 60 % 60, s % 60, millis % 1000)
}

/// `2026-11-15T08:03:06.042Z` or `2026-11-15T08:03:06Z` as milliseconds since 1970 (only the
/// forms `rfc3339` writes and wrote).
fn parse_rfc3339(text: &str) -> Option<u64> {
    let b = text.as_bytes();
    let millis = match b.len() {
        20 => 0,
        24 if b[19] == b'.' => text.get(20..23)?.parse::<u64>().ok()?,
        _ => return None,
    };
    if b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' || b[b.len() - 1] != b'Z' {
        return None;
    }
    let n = |r: std::ops::Range<usize>| text.get(r)?.parse::<i64>().ok();
    let (year, month, day) = (n(0..4)?, n(5..7)?, n(8..10)?);
    // Civil date to days (Howard Hinnant's algorithm).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = u64::try_from(days * 86_400 + n(11..13)? * 3600 + n(14..16)? * 60 + n(17..19)?).ok()?;
    Some(secs * 1000 + millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_utc_times() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(rfc3339(951_782_400_000), "2000-02-29T00:00:00.000Z");
        assert_eq!(rfc3339(1_794_729_786_042), "2026-11-15T08:03:06.042Z");
        for millis in [0, 951_782_400_000, 1_794_729_786_042, 4_102_444_799_999] {
            assert_eq!(parse_rfc3339(&rfc3339(millis)), Some(millis));
        }
        assert_eq!(parse_rfc3339("2026-11-15T08:03:06Z"), Some(1_794_729_786_000));
        assert_eq!(parse_rfc3339("2026-11-15 08:03:06"), None);
    }

    #[test]
    fn reports_become_otlp_log_records() {
        let lines = concat!(
            r#"{"time":"2026-11-15T08:03:06Z","device":"7f3a","os":"macos","osVersion":"15.1","furnish":"0.10.0","event":"install","outcome":"failed","app":"acme/vpn","collection":"acme","version":"c0ffee1","trigger":"user","error":"exit 1","durationMs":4200}"#, "\n",
            r#"{"time":"2026-11-15T08:03:07Z","device":"7f3a","os":"macos","osVersion":"15.1","furnish":"0.10.0","event":"inventory","outcome":"reported","apps":[{"app":"acme/vpn","status":"installed"}]}"#, "\n",
            r#"{"time":"2026-11-15T08:03:08Z","device":"7f3a","host":"mac-1","os":"macos","osVersion":"15.1","furnish":"0.10.0","event":"fallback","outcome":"succeeded","app":"acme/vpn","collection":"acme"}"#, "\n",
        );
        let request: serde_json::Value = serde_json::from_str(&otlp(lines)).unwrap();
        let resources = request["resourceLogs"].as_array().unwrap();
        assert_eq!(resources.len(), 2, "the host name makes another resource");
        let attributes = |v: &serde_json::Value| -> Vec<(String, serde_json::Value)> {
            v.as_array().unwrap().iter().map(|a| (a["key"].as_str().unwrap().to_string(), a["value"].clone())).collect()
        };
        let s = |v: &str| serde_json::json!({"stringValue": v});
        assert_eq!(
            attributes(&resources[0]["resource"]["attributes"]),
            [("service.name", s("furnish")), ("service.version", s("0.10.0")), ("os.type", s("darwin")), ("os.version", s("15.1")), ("device.id", s("7f3a"))]
                .map(|(k, v)| (k.to_string(), v))
        );
        assert_eq!(attributes(&resources[1]["resource"]["attributes"]).last().unwrap(), &("host.name".to_string(), s("mac-1")));
        let records = resources[0]["scopeLogs"][0]["logRecords"].as_array().unwrap();
        assert_eq!(records[0]["timeUnixNano"], "1794729786000000000");
        assert_eq!((records[0]["severityNumber"].as_u64(), records[0]["severityText"].as_str()), (Some(17), Some("ERROR")));
        assert_eq!(records[0]["body"], s("install acme/vpn failed"));
        assert_eq!(
            attributes(&records[0]["attributes"]),
            [
                ("furnish.event", s("install")),
                ("furnish.outcome", s("failed")),
                ("furnish.app", s("acme/vpn")),
                ("furnish.collection", s("acme")),
                ("furnish.collection.commit", s("c0ffee1")),
                ("furnish.trigger", s("user")),
                ("error.message", s("exit 1")),
                ("furnish.duration_ms", serde_json::json!({"intValue": "4200"})),
            ]
            .map(|(k, v)| (k.to_string(), v))
        );
        assert_eq!(records[1]["severityText"], "INFO");
        assert_eq!(records[1]["body"], s("inventory of 1 app"));
        assert_eq!(
            attributes(&records[1]["attributes"])[2],
            (
                "furnish.apps".to_string(),
                serde_json::json!({"arrayValue": {"values": [{"kvlistValue": {"values": [
                    {"key": "app", "value": s("acme/vpn")}, {"key": "status", "value": s("installed")}]}}]}})
            )
        );
        assert_eq!(resources[1]["scopeLogs"][0]["logRecords"][0]["severityText"], "WARN");
        assert!(!otlp(lines).contains("\"user."), "only the endpoint names the user");
    }

    #[test]
    fn requirement_reports_become_otlp_log_records() {
        let lines = concat!(
            r#"{"time":"2026-11-15T08:03:06Z","device":"7f3a","os":"windows","osVersion":"10","furnish":"0.10.0","event":"requirement","outcome":"notAcceptable","app":"acme/firefox","collection":"acme","installedVersion":"115.15.0","offeredVersion":"128.0.3"}"#, "\n",
            r#"{"time":"2026-11-15T08:03:07Z","device":"7f3a","os":"windows","osVersion":"10","furnish":"0.10.0","event":"requirement","outcome":"noneOffered","app":"acme/zoom","collection":"acme","installedVersion":"5.0","offeredVersion":"5.1"}"#, "\n",
            r#"{"time":"2026-11-15T08:03:08Z","device":"7f3a","os":"windows","osVersion":"10","furnish":"0.10.0","event":"requirement","outcome":"acceptable","app":"acme/firefox","collection":"acme"}"#, "\n",
        );
        let request: serde_json::Value = serde_json::from_str(&otlp(lines)).unwrap();
        let records = request["resourceLogs"][0]["scopeLogs"][0]["logRecords"].as_array().unwrap();
        let severity = |r: &serde_json::Value| (r["severityNumber"].as_u64().unwrap(), r["severityText"].as_str().unwrap().to_string());
        assert_eq!(records.iter().map(severity).collect::<Vec<_>>(), [(13, "WARN".into()), (17, "ERROR".into()), (9, "INFO".into())]);
        assert_eq!(records[0]["body"]["stringValue"], "requirement acme/firefox notAcceptable");
        let s = |v: &str| serde_json::json!({"stringValue": v});
        let attributes: Vec<(String, serde_json::Value)> =
            records[0]["attributes"].as_array().unwrap().iter().map(|a| (a["key"].as_str().unwrap().to_string(), a["value"].clone())).collect();
        assert_eq!(
            attributes,
            [
                ("furnish.event", s("requirement")),
                ("furnish.outcome", s("notAcceptable")),
                ("furnish.app", s("acme/firefox")),
                ("furnish.collection", s("acme")),
                ("furnish.installed_version", s("115.15.0")),
                ("furnish.offered_version", s("128.0.3")),
            ]
            .map(|(k, v)| (k.to_string(), v))
        );
        assert_eq!(records[2]["attributes"].as_array().unwrap().len(), 4);
    }

    fn required(installed: Option<&str>, acceptable: bool, offered: &str, target: Option<&str>) -> crate::required::Required {
        crate::required::Required {
            resource: "app".into(),
            installed: installed.map(Into::into),
            acceptable,
            offered: Some(offered.into()),
            via: "winget".into(),
            installer: false,
            target: target.map(Into::into),
            needs: vec![],
            summary: String::new(),
        }
    }

    #[test]
    fn only_requirement_changes_are_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("reports/requirements.json");
        // Each status check, and what it records: (outcome, installed, offered).
        let check = |app: &str, r: Option<crate::required::Required>, queued: bool| {
            let mut got = None;
            requirement_in(&state, app, "acme", Unmet::of(r.as_ref()), |e| {
                assert_eq!((e.event, e.app.as_deref(), e.collection.as_deref()), ("requirement", Some(app), Some("acme")));
                got = Some((e.outcome, e.installed_version, e.offered_version));
                queued
            });
            got
        };
        let some = |o: &'static str, i: &str, f: &str| Some((o, Some(i.to_string()), Some(f.to_string())));
        let old = || Some(required(Some("115.0"), false, "128.0.3", Some("128.0.3")));
        // No requirement, or met: nothing to record.
        assert_eq!(check("acme/firefox", None, true), None);
        assert_eq!(check("acme/firefox", Some(required(Some("128.0.3"), true, "128.0.3", Some("128.0.3"))), true), None);
        // Not installed: installing reports its own failure.
        assert_eq!(check("acme/firefox", Some(required(None, true, "127.0", None)), true), None);
        // Unacceptable: recorded once, again when the outcome or the installed version changes.
        assert_eq!(check("acme/firefox", old(), true), some("notAcceptable", "115.0", "128.0.3"));
        assert_eq!(check("acme/firefox", old(), true), None);
        assert_eq!(check("acme/firefox", Some(required(Some("115.0"), false, "128.0.4", Some("128.0.4"))), true), None);
        assert_eq!(check("acme/firefox", Some(required(Some("116.0"), false, "128.0.4", Some("128.0.4"))), true), some("notAcceptable", "116.0", "128.0.4"));
        assert_eq!(check("acme/firefox", Some(required(Some("116.0"), false, "128.0.4", None)), true), some("noneOffered", "116.0", "128.0.4"));
        // Another app has its own state.
        assert_eq!(check("acme/zoom", Some(required(Some("5.0"), false, "5.1", None)), true), some("noneOffered", "5.0", "5.1"));
        assert_eq!(check("acme/zoom", Some(required(Some("5.0"), false, "5.1", None)), true), None);
        // Acceptable again: once.
        assert_eq!(check("acme/firefox", None, true), Some(("acceptable", None, None)));
        assert_eq!(check("acme/firefox", None, true), None);
        // Uninstalled while unacceptable: no longer on that version.
        assert_eq!(check("acme/zoom", Some(required(None, true, "5.1", None)), true), Some(("acceptable", None, None)));
        // Not queued (reporting off): tried again at the next check.
        assert_eq!(check("acme/firefox", old(), false), some("notAcceptable", "115.0", "128.0.3"));
        assert_eq!(check("acme/firefox", old(), true), some("notAcceptable", "115.0", "128.0.3"));
        assert_eq!(check("acme/firefox", old(), true), None);
        // A damaged state file counts as nothing recorded.
        fs::write(&state, "{").unwrap();
        assert_eq!(check("acme/firefox", old(), true), some("notAcceptable", "115.0", "128.0.3"));
    }

    #[test]
    fn each_endpoint_gets_reports_about_its_collections_only() {
        let batch = concat!(
            r#"{"event":"install","collection":"acme","app":"acme/vpn"}"#, "\n",
            r#"{"event":"install","collection":"other","app":"other/x"}"#, "\n",
            r#"{"event":"inventory","apps":[{"app":"acme/vpn","status":"installed"},{"app":"other/x","status":"installed"}]}"#, "\n",
            r#"{"event":"inventory","apps":[{"app":"other/x","status":"installed"}]}"#, "\n",
        );
        assert_eq!(
            about(batch, &["acme"]),
            concat!(
                r#"{"event":"install","collection":"acme","app":"acme/vpn"}"#, "\n",
                r#"{"event":"inventory","apps":[{"app":"acme/vpn","status":"installed"}]}"#, "\n",
            )
        );
        assert_eq!(about(batch, &[]), "");
    }

    #[test]
    fn secrets_in_errors_are_redacted() {
        assert_eq!(
            redact("fatal: unable to access 'https://alice:s3cret@git.acme.tld/it/apps.git/': 401"),
            "fatal: unable to access 'https://***@git.acme.tld/it/apps.git/': 401"
        );
        assert_eq!(redact("https://ghp_abc@github.com/x"), "https://***@github.com/x");
        assert_eq!(redact("Authorization: Bearer abc.def-ghi"), "Authorization: Bearer ***");
        assert_eq!(redact("authorization: basic eDp5== rest"), "authorization: basic *** rest");
        assert_eq!(redact("token eyJhbGciOi.eyJzdWIi.c2ln was rejected"), "token *** was rejected");
        assert_eq!(redact("https://git.acme.tld/x.git: not found"), "https://git.acme.tld/x.git: not found");
        assert_eq!(redact("Grüße, user@example.com"), "Grüße, user@example.com");
    }

    #[test]
    fn batches_end_at_line_ends() {
        assert_eq!(first_batch("a\nb\n"), "a\nb\n");
        assert_eq!(first_batch("a\nb"), "a\n"); // a line still being written waits
        let long = format!("{}\nx\n", "y".repeat(MAX_BATCH + 10));
        assert_eq!(first_batch(&long).len(), MAX_BATCH + 11); // one oversized line alone
        // The limit counts the log records the lines become.
        let line = format!(r#"{{"event":"install","outcome":"failed","error":"{}"}}"#, "e".repeat(1000));
        let many = format!("{line}\n").repeat(2000);
        let batch = first_batch(&many);
        assert!(batch.len() < MAX_BATCH && batch.ends_with('\n'));
        assert!(otlp(batch).len() <= MAX_BATCH + 1024 && otlp(&many[..batch.len() + line.len() + 1]).len() > MAX_BATCH);
    }
}
