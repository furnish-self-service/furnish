//! Reports to the organization: what furnish installed, updated or removed on this computer and
//! whether it worked, sent to the `events` URL of the organization's furnish address (see
//! `endpoint`), whose proxy adds the signed-in user and passes them to the admins' log collector.
//!
//! The setting `reporting` decides what is recorded: `off` (default), `errors` (failures and
//! fallbacks) or `all` (also successes and a daily inventory). Events wait in
//! `reports/queue.ndjson` in the data directory until an endpoint accepts them, so offline
//! computers report later. Users can read that file; `furnish reports` shows it.

use std::{
    cell::Cell,
    fs,
    io::Write,
    path::PathBuf,
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
    /// install, update, uninstall, reinstall, fallback, collectionUpdate or inventory.
    pub event: &'static str,
    /// succeeded, failed or cancelled; `reported` for inventory.
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
    option_env!("FURNISH_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}

fn dir() -> PathBuf {
    crate::catalog::data_dir().join("reports")
}

pub fn queue_path() -> PathBuf {
    dir().join("queue.ndjson")
}

/// The queue keeps at most this much; older reports go first.
const MAX_QUEUE: u64 = 5 << 20;
/// At most this much per request (the endpoint's proxies accept 1 MiB).
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
    let important = event.outcome == "failed" || event.event == "fallback";
    let wanted = match level() {
        Level::Off => false,
        Level::Errors => important,
        Level::All => true,
    };
    if !wanted || IN_SERVICE.load(Ordering::Relaxed) {
        return;
    }
    if let Err(err) = append(&event) {
        eprintln!("warning: couldn't queue a report: {err:#}");
    }
}

fn append(event: &Event) -> Result<()> {
    let settings = crate::settings::load();
    let report = Report {
        time: rfc3339(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())),
        device: device_id()?,
        host: settings.report_hostname.then(sysinfo::System::host_name).flatten(),
        os: std::env::consts::OS.into(),
        os_version: sysinfo::System::os_version().unwrap_or_default(),
        furnish: version(),
        event,
    };
    fs::create_dir_all(dir())?;
    let path = queue_path();
    let mut file = fs::OpenOptions::new().create(true).append(true).open(&path)?;
    writeln!(file, "{}", serde_json::to_string(&report)?)?;
    if file.metadata()?.len() > MAX_QUEUE {
        let text = fs::read_to_string(&path)?;
        let keep = &text[text.len() / 2..];
        let keep = keep.find('\n').map_or("", |i| &keep[i + 1..]);
        fs::write(&path, keep)?;
    }
    Ok(())
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
    fs::create_dir_all(dir())?;
    fs::write(&path, &id)?;
    Ok(id)
}

/// Where reports go: the `events` URL of each known endpoint, if it is on the endpoint itself (the
/// user's token only ever goes there).
fn destinations() -> Vec<(String, crate::endpoint::Description, String)> {
    crate::endpoint::known()
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
/// ponytail: one sender at a time per process; two processes may both send a batch (duplicates).
pub fn send() {
    static SENDING: Mutex<()> = Mutex::new(());
    let Ok(_one) = SENDING.try_lock() else { return };
    if IN_SERVICE.load(Ordering::Relaxed) {
        return;
    }
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
            let Ok(token) = crate::endpoint::token(origin, description) else { return };
            let sent = crate::endpoint::http()
                .post(url)
                .header("Authorization", &format!("Bearer {token}"))
                .header("Content-Type", "application/x-ndjson")
                .send(batch);
            if let Err(err) = sent {
                eprintln!("warning: couldn't send reports to {}: {err}", description.name);
                return;
            }
        }
        // Drop what was sent; reports queued meanwhile stay.
        let Ok(now) = fs::read_to_string(queue_path()) else { return };
        if let Some(rest) = now.strip_prefix(batch)
            && fs::write(queue_path(), rest).is_err()
        {
            return;
        }
    }
}

/// Whole lines from the start of the queue, at most MAX_BATCH bytes (one line at least).
fn first_batch(text: &str) -> &str {
    let mut end = 0;
    for (i, _) in text.match_indices('\n') {
        if i + 1 > MAX_BATCH && end > 0 {
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
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) > last + 86_400
}

/// Record which apps are installed (with `reporting: all`, once a day).
pub fn inventory(apps: Vec<Installed>) {
    record(Event { event: "inventory", outcome: "reported", apps: Some(apps), ..Default::default() });
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let _ = fs::write(dir().join("inventory-at"), now.to_string());
}

/// Seconds since 1970 as `2026-11-15T08:03:06Z`.
fn rfc3339(secs: u64) -> String {
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
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", s / 3600, s / 60 % 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_utc_times() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_794_729_786), "2026-11-15T08:03:06Z");
    }

    #[test]
    fn batches_end_at_line_ends() {
        assert_eq!(first_batch("a\nb\n"), "a\nb\n");
        assert_eq!(first_batch("a\nb"), "a\n"); // a line still being written waits
        let long = format!("{}\nx\n", "y".repeat(MAX_BATCH + 10));
        assert_eq!(first_batch(&long).len(), MAX_BATCH + 11); // one oversized line alone
    }
}
