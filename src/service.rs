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
//!   collections is open to every local user; only authorized callers see why it failed (the
//!   details go to the service's log).
//! - Connections are limited in number and time: a request must arrive within `IO_TIMEOUT`, and a
//!   client that doesn't read its answers loses them instead of holding up the operation.

use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{SyncSender, TrySendError, sync_channel},
    },
    time::{Duration, Instant},
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
    !in_service() && crate::policy::load().service.is_some()
}

/// This process is the furnish service (running as root or SYSTEM).
pub fn in_service() -> bool {
    IN_SERVICE.load(Ordering::Relaxed)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum Request {
    Install {
        app: String,
        values: Map<String, Value>,
        #[serde(default)]
        which: crate::catalog::Which,
        /// The caller's `osSpecific.nixos.installInto`: the service can't read their settings,
        /// which may be anywhere (`$XDG_DATA_HOME`). It only chooses between places the caller
        /// may install into anyway (src/nixos_apps.rs).
        #[serde(default)]
        install_into: Option<String>,
    },
    Uninstall { app: String },
    Reinstall {
        app: String,
        values: Map<String, Value>,
        #[serde(default)]
        install_into: Option<String>,
    },
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
    /// The versions of an app people can choose from (`catalog::App::versions`).
    Versions { app: String },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
enum Reply {
    Progress(Progress),
    Log { level: String, message: String },
    Updated { old: String, new: String },
    Problems(Vec<String>),
    Versions(Vec<crate::catalog::Version>),
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

/// The versions of an app the service installs that people can choose from.
pub fn versions(app: &str) -> Result<Vec<crate::catalog::Version>> {
    match call(&Request::Versions { app: app.into() }, &mut |_| {})? {
        Reply::Versions(v) => Ok(v),
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
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{FILE_GENERIC_READ, FILE_WRITE_DATA, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT};
    // Only what the pipe's ACL grants users; the server may identify us but not act as us.
    let pipe = std::fs::OpenOptions::new()
        .access_mode(FILE_GENERIC_READ | FILE_WRITE_DATA)
        .security_qos_flags(SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION)
        .open(socket_path())?;
    // Requests carry the user's credentials: send them to the furnish service only.
    windows::check_server(&pipe)?;
    Ok(pipe)
}

// --- Service: shared by the platforms

/// Before anything writes to the system directory: make sure only the service can change it.
pub fn prepare() -> Result<()> {
    IN_SERVICE.store(true, Ordering::Relaxed);
    crate::reports::running_as_service();
    #[cfg(windows)]
    windows::secure_system_dir(&crate::catalog::system_dir())?;
    Ok(())
}

/// Where a connection's replies go: a queue that a thread of the connection writes to the client.
/// `None` once the client went away or stopped reading.
type Out = Arc<Mutex<Option<SyncSender<String>>>>;

/// Where log lines of the running operation go: the client that started it.
static CURRENT: Mutex<Option<Out>> = Mutex::new(None);
/// One operation at a time.
static RUNNING: Mutex<()> = Mutex::new(());

/// How long a client may take to send its request, and to take each answer.
const IO_TIMEOUT: Duration = Duration::from_secs(30);
/// Open connections at most; more are closed right away.
const MAX_CONNECTIONS: usize = 64;
/// Answers waiting for a client; a client that lets more pile up gets no more.
const QUEUE: usize = 1024;
static CONNECTIONS: AtomicUsize = AtomicUsize::new(0);

/// Never blocks: the operation goes on whether or not the client reads.
fn send(out: &Out, reply: &Reply) {
    let mut queue = out.lock().unwrap_or_else(|e| e.into_inner());
    let Some(q) = queue.as_ref() else { return };
    match q.try_send(serde_json::to_string(reply).unwrap_or_default()) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => {
            eprintln!("warning: a client doesn't read the service's answers; it gets no more of them");
            *queue = None;
        }
        Err(TrySendError::Disconnected(_)) => *queue = None,
    }
}

/// Reads that must all be done by a deadline (`set` sets the timeout of the next read).
struct Deadline<R> {
    inner: R,
    until: Instant,
    set: fn(&R, Duration) -> std::io::Result<()>,
}

impl<R: Read> Read for Deadline<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let left = self.until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        (self.set)(&self.inner, left)?;
        self.inner.read(buf)
    }
}

/// Answer a connection on threads of its own: one runs the request, one writes the answers
/// (`finish` gets the writer when all are written). Refused when too many are open or no thread
/// can be started. `authorize(interactive)`: may the caller install (`interactive`: ask for a
/// password if the OS's rules say so)?
#[cfg_attr(not(any(target_os = "linux", target_os = "macos", windows)), allow(dead_code))]
fn spawn_connection<R, W>(input: R, mut output: W, finish: fn(W), caller: Option<u32>, authorize: impl Fn(bool) -> Result<()> + Send + 'static)
where
    R: Read + Send + 'static,
    W: Write + Send + 'static,
{
    struct Slot;
    impl Drop for Slot {
        fn drop(&mut self) {
            CONNECTIONS.fetch_sub(1, Ordering::SeqCst);
        }
    }
    if CONNECTIONS.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
        CONNECTIONS.fetch_sub(1, Ordering::SeqCst);
        eprintln!("warning: {MAX_CONNECTIONS} connections are open; closed a new one");
        return;
    }
    let slot = Arc::new(Slot);
    let (queue, answers) = sync_channel::<String>(QUEUE);
    let writer_slot = slot.clone();
    let writer = std::thread::Builder::new().name("furnish-out".into()).spawn(move || {
        let _slot = writer_slot;
        // Ends when the request is answered (all senders gone), or when the client stops reading.
        for line in answers {
            if output.write_all(format!("{line}\n").as_bytes()).and_then(|()| output.flush()).is_err() {
                return;
            }
        }
        finish(output);
    });
    if let Err(err) = writer {
        return eprintln!("warning: can't answer a connection: {err}");
    }
    let out: Out = Arc::new(Mutex::new(Some(queue)));
    let handler = std::thread::Builder::new().name("furnish-request".into()).spawn(move || {
        let _slot = slot;
        handle(input, out, caller, &authorize);
    });
    if let Err(err) = handler {
        eprintln!("warning: can't answer a connection: {err}");
    }
}

/// Who asked for the running operation (uid), for Furnish/NixOS: it rebuilds a configuration that
/// belongs to a user only when that user asks (src/nixos_apps.rs). The variable is set from the
/// start, so later changes replace its value in place (as `resource_env::refresh`), under `RUNNING`.
const REQUESTER: &str = "FURNISH_REQUESTER_UID";

/// The caller's `installInto` for the running operation (`Request::Install`), for Furnish/NixOS.
pub const REQUESTER_INSTALL_INTO: &str = "FURNISH_REQUESTER_INSTALL_INTO";

fn set_requester(uid: Option<u32>) {
    // SAFETY: the name exists since `start`; only the operation holding RUNNING changes it.
    unsafe { std::env::set_var(REQUESTER, uid.map(|u| u.to_string()).unwrap_or_default()) };
}

fn set_install_into(value: Option<&str>) {
    let value = value.filter(|v| matches!(*v, "system" | "home")).unwrap_or_default();
    // SAFETY: as `set_requester`.
    unsafe { std::env::set_var(REQUESTER_INSTALL_INTO, value) };
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
fn start() {
    IN_SERVICE.store(true, Ordering::Relaxed);
    set_requester(None);
    set_install_into(None);
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
fn handle(input: impl Read, out: Out, caller: Option<u32>, authorize: &dyn Fn(bool) -> Result<()>) {
    let mut line = String::new();
    let reply = match BufReader::new(input.take(64 * 1024)).read_line(&mut line).context("reading the request").and_then(|_| {
        serde_json::from_str::<Request>(&line).context("invalid request")
    }) {
        Ok(request) => answer(request, &out, caller, authorize).unwrap_or_else(|e| {
            Reply::Error(if e.is::<engine::Cancelled>() { engine::CANCELLED.into() } else { format!("{e:#}") })
        }),
        Err(e) => Reply::Error(format!("{e:#}")),
    };
    send(&out, &reply);
}

fn answer(request: Request, out: &Out, caller: Option<u32>, authorize: &dyn Fn(bool) -> Result<()>) -> Result<Reply> {
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
    let operation = |work: Work, install_into: Option<&str>| -> Result<Reply> {
        let _one_at_a_time = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(out.clone());
        set_requester(caller);
        set_install_into(install_into);
        let result = work(&mut |p| send(out, &Reply::Progress(p)));
        set_requester(None);
        set_install_into(None);
        *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = None;
        result.map(|()| Reply::Done)
    };
    // Every user may set up and update the preset collections (the app of each user does), but
    // only those allowed to install see why it failed; the details go to the service's log.
    let trusted = || authorize(false).is_ok();
    match request {
        // Fetching with the user's sign-in: the service only ever installs what it fetched.
        Request::EnsurePresets { headers } => {
            let problems = crate::auth::with_headers(headers, crate::catalog::ensure_presets);
            if problems.is_empty() || trusted() {
                return Ok(Reply::Problems(problems));
            }
            for problem in &problems {
                eprintln!("warning: {problem}");
            }
            return Ok(Reply::Problems(vec![HIDDEN.replace("{what}", "set up your organization's collections")]));
        }
        Request::Update { collection, headers } => {
            let c = crate::catalog::get(&collection)?;
            if !c.managed {
                bail!("'{collection}' isn't a collection your organization provides");
            }
            return match crate::auth::with_headers(headers, || c.update()) {
                Ok((old, new)) => Ok(Reply::Updated { old, new }),
                Err(err) if trusted() => Err(err),
                Err(err) => {
                    eprintln!("warning: updating {collection}: {err:#}");
                    bail!(HIDDEN.replace("{what}", &format!("update '{collection}'")))
                }
            };
        }
        // Reading, like updating: every user may see which versions there are.
        Request::Versions { app: key } => {
            return match app(&key).and_then(|app| app.versions()) {
                Ok(versions) => Ok(Reply::Versions(versions)),
                Err(err) if trusted() => Err(err),
                Err(err) => {
                    eprintln!("warning: versions of {key}: {err:#}");
                    bail!(HIDDEN.replace("{what}", &format!("list the versions of '{key}'")))
                }
            };
        }
        _ => authorize(true)?,
    }
    match request {
        Request::Install { app: key, values, which, install_into } => {
            let app = app(&key)?;
            let values = accept(&app, &values)?;
            operation(&|report| app.install_here(&values, which.clone(), report), install_into.as_deref())
        }
        Request::Uninstall { app: key } => {
            let app = app(&key)?;
            operation(&|report| app.uninstall(report), None)
        }
        Request::Reinstall { app: key, values, install_into } => {
            let app = app(&key)?;
            let values = accept(&app, &values)?;
            operation(&|report| app.reinstall_with(&values, report), install_into.as_deref())
        }
        Request::Stop { force } => {
            if force { engine::force_stop() } else { engine::abort() }
            Ok(Reply::Done)
        }
        Request::Update { .. } | Request::EnsurePresets { .. } | Request::Versions { .. } => unreachable!("answered above"),
    }
}

const REFUSED: &str = "you aren't allowed to install your organization's apps on this computer; ask your administrator";
/// What callers who may not install learn about a failure.
const HIDDEN: &str = "the furnish service couldn't {what}; your administrator finds the details in its log";

/// A user's name and primary group.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn user(uid: u32) -> Result<(std::ffi::CString, libc::gid_t)> {
    let mut buf = vec![0 as libc::c_char; 16 * 1024];
    // SAFETY: zeroed passwd is a valid out-parameter; getpwuid_r fills it with pointers into buf.
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    let rc = unsafe { libc::getpwuid_r(uid, &mut pw, buf.as_mut_ptr(), buf.len(), &mut found) };
    if rc != 0 || found.is_null() {
        bail!("there's no user with the id {uid}");
    }
    // SAFETY: pw_name points to a NUL-terminated string in buf.
    Ok((unsafe { std::ffi::CStr::from_ptr(pw.pw_name) }.to_owned(), pw.pw_gid))
}

/// Whether a user (uid) is in the policy's `allowedGroup`, as the system's user databases say
/// (NSS: also directory groups, e.g. through SSSD).
#[cfg(target_os = "linux")]
fn in_allowed_group(uid: u32) -> Result<bool> {
    let Some(group) = crate::policy::load().service.and_then(|s| s.allowed_group) else { return Ok(false) };
    let Ok(group) = std::ffi::CString::new(group) else { return Ok(false) };
    let mut buf = vec![0 as libc::c_char; 64 * 1024];
    // SAFETY: zeroed group is a valid out-parameter; getgrnam_r fills it with pointers into buf.
    let mut gr: libc::group = unsafe { std::mem::zeroed() };
    let mut found: *mut libc::group = std::ptr::null_mut();
    if unsafe { libc::getgrnam_r(group.as_ptr(), &mut gr, buf.as_mut_ptr(), buf.len(), &mut found) } != 0 || found.is_null() {
        return Ok(false);
    }
    let (name, primary) = user(uid)?;
    let mut groups: Vec<libc::gid_t> = vec![0; 256];
    loop {
        let mut n = groups.len() as libc::c_int;
        // SAFETY: groups has room for n entries; getgrouplist sets n to how many there are.
        let rc = unsafe { libc::getgrouplist(name.as_ptr(), primary, groups.as_mut_ptr(), &mut n) };
        if rc >= 0 {
            groups.truncate(n.max(0) as usize);
            return Ok(groups.contains(&gr.gr_gid));
        }
        if groups.len() >= 1 << 16 {
            bail!("can't read the groups of {}", name.to_string_lossy());
        }
        groups.resize((n.max(0) as usize).max(groups.len() * 2), 0);
    }
}

/// Whether a user (uid) is in the policy's `allowedGroup`; dseditgroup also resolves nested and
/// directory (e.g. Active Directory) groups.
#[cfg(target_os = "macos")]
fn in_allowed_group(uid: u32) -> Result<bool> {
    let Some(group) = crate::policy::load().service.and_then(|s| s.allowed_group) else { return Ok(false) };
    let (name, _) = user(uid)?;
    let member = std::process::Command::new("/usr/sbin/dseditgroup")
        .args(["-o", "checkmember", "-m", &name.to_string_lossy(), &group])
        .stdout(std::process::Stdio::null())
        .status()?;
    Ok(member.success())
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
                // E.g. out of file descriptors: give connections time to close.
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
        };
        let peer = peer(&stream);
        let output = match stream.set_write_timeout(Some(IO_TIMEOUT)).and_then(|()| stream.try_clone()) {
            Ok(s) => s,
            Err(err) => {
                eprintln!("warning: {err}");
                continue;
            }
        };
        let input = Deadline { inner: stream, until: Instant::now() + IO_TIMEOUT, set: |s, left| s.set_read_timeout(Some(left)) };
        let caller = peer.as_ref().ok().map(|p| p.uid);
        spawn_connection(input, output, drop, caller, move |interactive| {
            authorize(peer.as_ref().map_err(|e| anyhow::anyhow!("{e:#}"))?, interactive)
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
fn authorize(peer: &Peer, _interactive: bool) -> Result<()> {
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
fn authorize(peer: &Peer, interactive: bool) -> Result<()> {
    if peer.uid == 0 || in_allowed_group(peer.uid)? {
        return Ok(());
    }
    // polkit asks the user's session for a password if its rules say so and `interactive` (pid +
    // start time, so a reused pid can't borrow another process's authorization).
    let stat = std::fs::read_to_string(format!("/proc/{}/stat", peer.pid))?;
    let start = stat.rsplit_once(')').and_then(|(_, rest)| rest.split_whitespace().nth(19)).context("reading the process start time")?;
    let process = format!("{},{start},{}", peer.pid, peer.uid);
    let mut args = vec!["--action-id", POLKIT_ACTION, "--process", &process];
    if interactive {
        args.push("--allow-user-interaction");
    }
    let allowed = std::process::Command::new("pkcheck")
        .args(args)
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

/// Only administrators can change this folder or what is in it: the same rule as for the furnish
/// binary that `service install` registers. Unreadable counts as changeable.
pub fn admin_only(dir: &std::path::Path) -> bool {
    matches!(install::changeable(dir), Ok(None))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn serve() -> Result<()> {
    bail!("the furnish service isn't available on this OS yet")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_client_that_doesnt_read_loses_its_answers_without_holding_up_the_service() {
        let (queue, answers) = sync_channel(2);
        let out: Out = Arc::new(Mutex::new(Some(queue)));
        for _ in 0..10 {
            send(&out, &Reply::Done); // returns at once although nobody reads
        }
        assert!(out.lock().unwrap().is_none(), "the client still gets answers");
        assert_eq!(answers.try_iter().count(), 2);
    }

    #[test]
    fn a_request_must_arrive_in_time() {
        /// Sends a byte every 20 ms, forever.
        struct Trickle;
        impl Read for Trickle {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                std::thread::sleep(Duration::from_millis(20));
                buf[0] = b' ';
                Ok(1)
            }
        }
        let input = Deadline { inner: Trickle, until: Instant::now() + Duration::from_millis(100), set: |_, _| Ok(()) };
        let err = BufReader::new(input).read_line(&mut String::new()).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn users_are_looked_up_by_id_not_by_name() {
        assert_eq!(user(0).unwrap().0.to_str().unwrap(), "root");
        assert!(user(u32::MAX - 1).is_err());
    }
}
