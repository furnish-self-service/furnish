//! The furnish service: installs apps of the admin's preset collections system-wide for users
//! without administrator rights. Enabled by a `service` section in the admin policy; without it,
//! furnish runs everything as the user and asks for elevation per step (small orgs).
//!
//! `furnish service` runs as root and listens on a Unix socket (`/run/furnish.sock`;
//! `FURNISH_SOCKET` overrides it). Clients send one JSON request per connection and get JSON
//! lines back: progress and log lines, then the result. The trust boundary:
//!
//! - Requests name an app and option values, never files or DSC code. The service only runs
//!   apps from its own clones of the preset collections, in its data directory (`system_dir`),
//!   which users can read but not change; with `trustedKeys`, only signed commits.
//! - Option values are checked against the app's declarations and the policy (`options::accept`).
//! - Installing, uninstalling and stopping need authorization: root, a member of the policy's
//!   `allowedGroup`, or what polkit allows for `io.github.furnish.install` (by default an
//!   administrator password). Updating the preset collections is open to every local user.

use std::{
    io::{BufRead, BufReader, Read, Write},
    sync::atomic::{AtomicBool, Ordering},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::engine::{self, Progress};

/// polkit action that authorizes installing through the service.
pub const POLKIT_ACTION: &str = "io.github.furnish.install";

static IN_SERVICE: AtomicBool = AtomicBool::new(false);

/// The admin policy routes preset collections through the service (and this isn't the service).
pub fn enabled() -> bool {
    !IN_SERVICE.load(Ordering::Relaxed) && crate::policy::load().service.is_some()
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum Request {
    Install { app: String, values: Map<String, Value> },
    Uninstall { app: String },
    Reinstall { app: String, values: Map<String, Value> },
    /// Stop the running operation after its step, or (force) now.
    Stop { force: bool },
    Update { collection: String },
    EnsurePresets,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
enum Reply {
    Progress(Progress),
    Log { level: String, message: String },
    Updated { old: String, new: String },
    Problems(Vec<String>),
    Done,
    Error(String),
}

fn socket_path() -> std::path::PathBuf {
    std::env::var_os("FURNISH_SOCKET").map_or_else(|| "/run/furnish.sock".into(), Into::into)
}

// --- Client

/// Run an install/uninstall/reinstall through the service, reporting its progress and logs.
pub fn run(request: &Request, mut report: impl FnMut(Progress)) -> Result<()> {
    match call(request, &mut report)? {
        Reply::Done => Ok(()),
        other => bail!("unexpected answer from the furnish service: {other:?}"),
    }
}

pub fn update(collection: &str) -> Result<(String, String)> {
    match call(&Request::Update { collection: collection.into() }, &mut |_| {})? {
        Reply::Updated { old, new } => Ok((old, new)),
        other => bail!("unexpected answer from the furnish service: {other:?}"),
    }
}

pub fn ensure_presets() -> Result<Vec<String>> {
    match call(&Request::EnsurePresets, &mut |_| {})? {
        Reply::Problems(p) => Ok(p),
        other => bail!("unexpected answer from the furnish service: {other:?}"),
    }
}

/// Ask the service to stop what it is running (no-op without the service).
pub fn stop(force: bool) -> Result<()> {
    if enabled() {
        call(&Request::Stop { force }, &mut |_| {})?;
    }
    Ok(())
}

#[cfg(unix)]
fn call(request: &Request, report: &mut dyn FnMut(Progress)) -> Result<Reply> {
    let mut stream = std::os::unix::net::UnixStream::connect(socket_path())
        .context("can't reach the furnish service, which installs your organization's apps; ask your administrator")?;
    stream.write_all(format!("{}\n", serde_json::to_string(request)?).as_bytes())?;
    for line in BufReader::new(stream).lines() {
        match serde_json::from_str(&line?).context("reading the furnish service's answer")? {
            Reply::Progress(p) => report(p),
            Reply::Log { level, message } => engine::emit_log(&level, message),
            Reply::Error(e) if e == engine::CANCELLED => return Err(engine::Cancelled.into()),
            Reply::Error(e) => bail!("{e}"),
            reply => return Ok(reply),
        }
    }
    bail!("the furnish service stopped before finishing")
}

#[cfg(not(unix))]
fn call(_: &Request, _: &mut dyn FnMut(Progress)) -> Result<Reply> {
    bail!("the furnish service isn't available on this OS yet")
}

// --- Service

#[cfg(target_os = "linux")]
pub fn serve() -> Result<()> {
    use std::{
        os::unix::{fs::PermissionsExt, net::UnixListener},
        sync::{Arc, Mutex},
    };
    IN_SERVICE.store(true, Ordering::Relaxed);
    for problem in crate::catalog::ensure_presets() {
        eprintln!("warning: {problem}");
    }
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).with_context(|| format!("listening on {}", path.display()))?;
    // Anyone may connect; each request is authorized by who sent it.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666))?;
    eprintln!("furnish service listening on {}", path.display());

    // Log lines of the running operation go to the client that started it.
    type Out = Arc<Mutex<std::os::unix::net::UnixStream>>;
    static CURRENT: Mutex<Option<Out>> = Mutex::new(None);
    static RUNNING: Mutex<()> = Mutex::new(());
    fn send(out: &Out, reply: &Reply) {
        let mut s = out.lock().unwrap_or_else(|e| e.into_inner());
        // A client that went away doesn't stop the operation.
        let _ = s.write_all(format!("{}\n", serde_json::to_string(reply).unwrap_or_default()).as_bytes());
    }
    engine::capture_logs(|line| {
        if let Some(out) = CURRENT.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            send(out, &Reply::Log { level: line.level.into(), message: line.message });
        }
    });

    fn handle(stream: std::os::unix::net::UnixStream) -> Result<()> {
        let peer = peer(&stream)?;
        let mut line = String::new();
        BufReader::new((&stream).take(64 * 1024)).read_line(&mut line)?;
        let out: Out = Arc::new(Mutex::new(stream));
        let reply = match serde_json::from_str::<Request>(&line).context("invalid request") {
            Ok(request) => answer(request, &peer, &out).unwrap_or_else(|e| {
                Reply::Error(if e.is::<engine::Cancelled>() { engine::CANCELLED.into() } else { format!("{e:#}") })
            }),
            Err(e) => Reply::Error(format!("{e:#}")),
        };
        send(&out, &reply);
        Ok(())
    }

    fn answer(request: Request, peer: &Peer, out: &Out) -> Result<Reply> {
        let app = |key: &str| -> Result<crate::catalog::App> {
            let app = crate::catalog::find_app(key)?;
            if !app.managed {
                bail!("'{key}' isn't from a collection your organization provides");
            }
            Ok(app)
        };
        let accept = |app: &crate::catalog::App, values: &Map<String, Value>| crate::options::accept(&app.options(), values);
        type Work<'a> = &'a dyn Fn(&mut dyn FnMut(Progress)) -> Result<()>;
        let operation = |work: Work| -> Result<Reply> {
            let _one_at_a_time = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
            *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(out.clone());
            let result = work(&mut |p| send(out, &Reply::Progress(p)));
            *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = None;
            result.map(|()| Reply::Done)
        };
        match request {
            Request::EnsurePresets => return Ok(Reply::Problems(crate::catalog::ensure_presets())),
            Request::Update { collection } => {
                let c = crate::catalog::get(&collection)?;
                if !c.managed {
                    bail!("'{collection}' isn't a collection your organization provides");
                }
                let (old, new) = c.update()?;
                return Ok(Reply::Updated { old, new });
            }
            _ => authorize(peer)?,
        }
        match request {
            Request::Install { app: key, values } => {
                let app = app(&key)?;
                let values = accept(&app, &values)?;
                operation(&|report| app.install_with(&values, report))
            }
            Request::Uninstall { app: key } => {
                let app = app(&key)?;
                operation(&|report| app.uninstall(report))
            }
            Request::Reinstall { app: key, values } => {
                let app = app(&key)?;
                let values = accept(&app, &values)?;
                operation(&|report| app.reinstall_with(&values, report))
            }
            Request::Stop { force } => {
                if force { engine::force_stop() } else { engine::abort() }
                Ok(Reply::Done)
            }
            Request::Update { .. } | Request::EnsurePresets => unreachable!("answered above"),
        }
    }

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                std::thread::spawn(move || {
                    if let Err(err) = handle(stream) {
                        eprintln!("warning: {err:#}");
                    }
                });
            }
            Err(err) => eprintln!("warning: {err}"),
        }
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn serve() -> Result<()> {
    bail!("the furnish service isn't available on this OS yet")
}

/// Who is on the other end of the socket, as the kernel reports it.
#[cfg(target_os = "linux")]
struct Peer {
    pid: i32,
    uid: u32,
}

#[cfg(target_os = "linux")]
fn peer(stream: &std::os::unix::net::UnixStream) -> Result<Peer> {
    use std::os::fd::AsRawFd;
    let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: SO_PEERCRED fills a ucred of the given length.
    let rc = unsafe {
        libc::getsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, (&raw mut cred).cast(), &mut len)
    };
    if rc != 0 {
        bail!("can't tell who is asking: {}", std::io::Error::last_os_error());
    }
    Ok(Peer { pid: cred.pid, uid: cred.uid })
}

#[cfg(target_os = "linux")]
fn authorize(peer: &Peer) -> Result<()> {
    if peer.uid == 0 {
        return Ok(());
    }
    if let Some(group) = crate::policy::load().service.and_then(|s| s.allowed_group) {
        let groups = std::process::Command::new("id").args(["-nG", &peer.uid.to_string()]).output()?;
        if String::from_utf8_lossy(&groups.stdout).split_whitespace().any(|g| g == group) {
            return Ok(());
        }
    }
    // polkit asks the user's session for a password if its rules say so (pid + start time, so a
    // reused pid can't borrow another process's authorization).
    let stat = std::fs::read_to_string(format!("/proc/{}/stat", peer.pid))?;
    let start = stat.rsplit_once(')').and_then(|(_, rest)| rest.split_whitespace().nth(19)).context("reading the process start time")?;
    let allowed = std::process::Command::new("pkcheck")
        .args(["--action-id", POLKIT_ACTION, "--process", &format!("{},{start},{}", peer.pid, peer.uid), "--allow-user-interaction"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !allowed {
        bail!("you aren't allowed to install your organization's apps on this computer; ask your administrator");
    }
    Ok(())
}
