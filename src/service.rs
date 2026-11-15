//! The furnish service: installs apps of the admin's preset collections system-wide for users
//! without administrator rights. Enabled by a `service` section in the admin policy; without it,
//! furnish runs everything as the user and asks for elevation per step (small orgs).
//!
//! `furnish service` runs as root (Linux: systemd, Unix socket `/run/furnish.sock`; macOS: launchd
//! daemon, `/var/run/furnish.sock`) or as the Windows service `furnish` under SYSTEM (named pipe
//! `\\.\pipe\furnish`); `FURNISH_SOCKET` overrides the address. Clients send one JSON request per connection and get JSON lines back: progress and
//! log lines, then the result. The trust boundary:
//!
//! - Requests name an app and option values, never files or DSC code. The service only runs
//!   apps from its own clones of the preset collections, in its data directory (`system_dir`),
//!   which users can read but not change; with `trustedKeys`, only signed commits.
//! - Option values are checked against the app's declarations and the policy (`options::accept`).
//! - Installing, uninstalling and stopping need authorization: root/SYSTEM (or, on Windows, an
//!   elevated administrator), a member of the policy's `allowedGroup`, or (Linux) what polkit allows for
//!   `io.github.furnish.install` (by default an administrator password). Updating the preset
//!   collections is open to every local user.

use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
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
    Install {
        app: String,
        values: Map<String, Value>,
        #[serde(default)]
        which: crate::catalog::Which,
    },
    Uninstall { app: String },
    Reinstall { app: String, values: Map<String, Value> },
    /// Stop the running operation after its step, or (force) now.
    Stop { force: bool },
    /// `headers`: the user's credentials for the collections' servers (URL prefix →
    /// `Authorization: …`: endpoint sign-ins, stored passwords), which the service fetches with.
    Update {
        collection: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
    EnsurePresets {
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
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
    let default = if cfg!(windows) {
        r"\\.\pipe\furnish"
    } else if cfg!(target_os = "macos") {
        "/var/run/furnish.sock"
    } else {
        "/run/furnish.sock"
    };
    std::env::var_os("FURNISH_SOCKET").map_or_else(|| default.into(), Into::into)
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
    let request = Request::Update { collection: collection.into(), headers: preset_headers() };
    match call(&request, &mut |_| {})? {
        Reply::Updated { old, new } => Ok((old, new)),
        other => bail!("unexpected answer from the furnish service: {other:?}"),
    }
}

pub fn ensure_presets() -> Result<Vec<String>> {
    match call(&Request::EnsurePresets { headers: preset_headers() }, &mut |_| {})? {
        Reply::Problems(p) => Ok(p),
        other => bail!("unexpected answer from the furnish service: {other:?}"),
    }
}

/// The user's credentials for the organization's collections, for the service to fetch with.
fn preset_headers() -> BTreeMap<String, String> {
    crate::auth::headers_for(crate::policy::load().collections.preset.into_iter().map(|p| p.url))
}

/// Ask the service to stop what it is running (no-op without the service).
pub fn stop(force: bool) -> Result<()> {
    if enabled() {
        call(&Request::Stop { force }, &mut |_| {})?;
    }
    Ok(())
}

fn call(request: &Request, report: &mut dyn FnMut(Progress)) -> Result<Reply> {
    let mut stream = connect()
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

#[cfg(unix)]
fn connect() -> std::io::Result<std::os::unix::net::UnixStream> {
    std::os::unix::net::UnixStream::connect(socket_path())
}

#[cfg(windows)]
fn connect() -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new().read(true).write(true).open(socket_path())
}

// --- Service: shared by the platforms

/// Before anything writes to the system directory: make sure only the service can change it.
pub fn prepare() -> Result<()> {
    crate::reports::running_as_service();
    #[cfg(windows)]
    windows::secure_system_dir(&crate::catalog::system_dir())?;
    Ok(())
}

type Out = Arc<Mutex<Box<dyn Write + Send>>>;

/// Where log lines of the running operation go: the client that started it.
static CURRENT: Mutex<Option<Out>> = Mutex::new(None);
/// One operation at a time.
static RUNNING: Mutex<()> = Mutex::new(());

fn send(out: &Out, reply: &Reply) {
    let mut s = out.lock().unwrap_or_else(|e| e.into_inner());
    // A client that went away doesn't stop the operation.
    let _ = s.write_all(format!("{}\n", serde_json::to_string(reply).unwrap_or_default()).as_bytes());
    let _ = s.flush();
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
fn start() {
    IN_SERVICE.store(true, Ordering::Relaxed);
    engine::capture_logs(|line| {
        if let Some(out) = CURRENT.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            send(out, &Reply::Log { level: line.level.into(), message: line.message });
        }
    });
    for problem in crate::catalog::ensure_presets() {
        eprintln!("warning: {problem}");
    }
}

/// Answer one connection: read the request, check it, run it, send the result.
#[cfg_attr(not(any(target_os = "linux", target_os = "macos", windows)), allow(dead_code))]
fn handle(input: impl Read, out: Out, authorize: &dyn Fn() -> Result<()>) {
    let mut line = String::new();
    let reply = match BufReader::new(input.take(64 * 1024)).read_line(&mut line).context("reading the request").and_then(|_| {
        serde_json::from_str::<Request>(&line).context("invalid request")
    }) {
        Ok(request) => answer(request, &out, authorize).unwrap_or_else(|e| {
            Reply::Error(if e.is::<engine::Cancelled>() { engine::CANCELLED.into() } else { format!("{e:#}") })
        }),
        Err(e) => Reply::Error(format!("{e:#}")),
    };
    send(&out, &reply);
}

fn answer(request: Request, out: &Out, authorize: &dyn Fn() -> Result<()>) -> Result<Reply> {
    let app = |key: &str| -> Result<crate::catalog::App> {
        let app = crate::catalog::find_app(key)?;
        if !app.managed {
            bail!("'{key}' isn't from a collection your organization provides");
        }
        Ok(app)
    };
    let accept = |app: &crate::catalog::App, values: &Map<String, Value>| {
        let values = crate::options::accept(&app.options(), values)?;
        if app.per_user(&values) {
            bail!("with these options {} installs into your user account, which furnish does without the service", app.id);
        }
        Ok(values)
    };
    type Work<'a> = &'a dyn Fn(&mut dyn FnMut(Progress)) -> Result<()>;
    let operation = |work: Work| -> Result<Reply> {
        let _one_at_a_time = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(out.clone());
        let result = work(&mut |p| send(out, &Reply::Progress(p)));
        *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = None;
        result.map(|()| Reply::Done)
    };
    match request {
        // Fetching with the user's sign-in: the service only ever installs what it fetched.
        Request::EnsurePresets { headers } => {
            return Ok(Reply::Problems(crate::auth::with_headers(headers, crate::catalog::ensure_presets)));
        }
        Request::Update { collection, headers } => {
            let c = crate::catalog::get(&collection)?;
            if !c.managed {
                bail!("'{collection}' isn't a collection your organization provides");
            }
            let (old, new) = crate::auth::with_headers(headers, || c.update())?;
            return Ok(Reply::Updated { old, new });
        }
        _ => authorize()?,
    }
    match request {
        Request::Install { app: key, values, which } => {
            let app = app(&key)?;
            let values = accept(&app, &values)?;
            operation(&|report| app.install_here(&values, which, report))
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
        Request::Update { .. } | Request::EnsurePresets { .. } => unreachable!("answered above"),
    }
}

const REFUSED: &str = "you aren't allowed to install your organization's apps on this computer; ask your administrator";

/// Whether a user (uid) is in the policy's `allowedGroup`.
#[cfg(target_os = "linux")]
fn in_allowed_group(uid: u32) -> Result<bool> {
    let Some(group) = crate::policy::load().service.and_then(|s| s.allowed_group) else { return Ok(false) };
    let groups = std::process::Command::new("id").args(["-nG", &uid.to_string()]).output()?;
    Ok(String::from_utf8_lossy(&groups.stdout).split_whitespace().any(|g| g == group))
}

/// Whether a user (uid) is in the policy's `allowedGroup`; dseditgroup also resolves nested and
/// directory (e.g. Active Directory) groups.
#[cfg(target_os = "macos")]
fn in_allowed_group(uid: u32) -> Result<bool> {
    let Some(group) = crate::policy::load().service.and_then(|s| s.allowed_group) else { return Ok(false) };
    let name = std::process::Command::new("id").args(["-un", &uid.to_string()]).output()?;
    let name = String::from_utf8_lossy(&name.stdout).trim().to_string();
    let member = std::process::Command::new("dseditgroup")
        .args(["-o", "checkmember", "-m", &name, &group])
        .stdout(std::process::Stdio::null())
        .status()?;
    Ok(!name.is_empty() && member.success())
}

// --- Service on Linux and macOS: Unix socket, caller from the kernel

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn serve() -> Result<()> {
    use std::os::unix::{fs::PermissionsExt, net::UnixListener};
    start();
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).with_context(|| format!("listening on {}", path.display()))?;
    // Anyone may connect; each request is authorized by who sent it.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666))?;
    eprintln!("furnish service listening on {}", path.display());
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(err) => {
                eprintln!("warning: {err}");
                continue;
            }
        };
        std::thread::spawn(move || {
            let peer = peer(&stream);
            let out: Out = match stream.try_clone() {
                Ok(s) => Arc::new(Mutex::new(Box::new(s))),
                Err(err) => return eprintln!("warning: {err}"),
            };
            handle(&stream, out, &|| authorize(peer.as_ref().map_err(|e| anyhow::anyhow!("{e:#}"))?));
        });
    }
    Ok(())
}

/// Who is on the other end of the socket, as the kernel reports it.
#[cfg(any(target_os = "linux", target_os = "macos"))]
struct Peer {
    #[cfg(target_os = "linux")]
    pid: i32,
    uid: u32,
}

#[cfg(target_os = "macos")]
fn peer(stream: &std::os::unix::net::UnixStream) -> Result<Peer> {
    use std::os::fd::AsRawFd;
    let (mut uid, mut gid) = (0, 0);
    // SAFETY: getpeereid fills two integers.
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
        bail!("can't tell who is asking: {}", std::io::Error::last_os_error());
    }
    Ok(Peer { uid })
}

/// macOS: root (sudo) or a member of `allowedGroup`.
#[cfg(target_os = "macos")]
fn authorize(peer: &Peer) -> Result<()> {
    if peer.uid == 0 || in_allowed_group(peer.uid)? {
        return Ok(());
    }
    bail!(REFUSED)
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
    if peer.uid == 0 || in_allowed_group(peer.uid)? {
        return Ok(());
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
        bail!(REFUSED);
    }
    Ok(())
}

// --- Service on Windows: a Windows service with a named pipe, caller from its token

#[cfg(windows)]
pub fn serve() -> Result<()> {
    windows::serve()
}

#[cfg(windows)]
mod windows;

mod install;
pub use install::{install, uninstall};

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn serve() -> Result<()> {
    bail!("the furnish service isn't available on this OS yet")
}
