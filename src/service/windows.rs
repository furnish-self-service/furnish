//! The furnish service on Windows: the Windows service `furnish`, running as SYSTEM, answers on
//! the named pipe `\\.\pipe\furnish`. Callers are identified by impersonating them.

use std::{
    ffi::OsString,
    fs::File,
    os::windows::io::AsRawHandle,
    path::Path,
    ptr::{null, null_mut},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use windows_service::{
    define_windows_service,
    service::{ServiceAccess, ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType},
    service_control_handler::{self, ServiceControlHandlerResult, ServiceStatusHandle},
    service_dispatcher,
    service_manager::{ServiceManager, ServiceManagerAccess},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_BROKEN_PIPE, ERROR_FAILED_SERVICE_CONTROLLER_CONNECT, ERROR_IO_PENDING, ERROR_OPERATION_ABORTED, ERROR_PIPE_CONNECTED,
        HANDLE, INVALID_HANDLE_VALUE, LocalFree, WAIT_OBJECT_0,
    },
    Security::{
        ACCESS_ALLOWED_ACE, ACE_HEADER, ACL,
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, ConvertStringSidToSidW, GetNamedSecurityInfoW,
            GetSecurityInfo, SDDL_REVISION_1, SE_FILE_OBJECT, SetNamedSecurityInfoW,
        },
        CheckTokenMembership, DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetSecurityDescriptorControl, GetSecurityDescriptorDacl,
        GetSecurityDescriptorOwner, INHERIT_ONLY_ACE, LookupAccountNameW, OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR, PSID, RevertToSelf, SE_DACL_PROTECTED, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    },
    Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CreateDirectoryW, CreateFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OPEN_REPARSE_POINT, FILE_FLAG_OVERLAPPED, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, READ_CONTROL, ReadFile, WriteFile,
    },
    System::{
        IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
        Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeServerProcessId, ImpersonateNamedPipeClient,
            PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
        },
        Threading::{CreateEventW, GetCurrentThread, OpenThreadToken, WaitForSingleObject},
    },
};

const NAME: &str = "furnish";
const SYSTEM: &str = "S-1-5-18";
const ADMINISTRATORS: &str = "S-1-5-32-544";
const TRUSTED_INSTALLER: &str = "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";
/// SYSTEM and Administrators have full control, users may read; protected from inheriting
/// ProgramData's "users may create files".
const DIR_SDDL: &str = "O:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FRFX;;;BU)";
/// The service's log: details of failures that only administrators should see.
const LOG_SDDL: &str = "D:P(A;;FA;;;SY)(A;;FA;;;BA)";
/// Authenticated users may connect: read, and write data (0x12008b: FILE_GENERIC_READ |
/// FILE_WRITE_DATA). Not FILE_APPEND_DATA, which on a pipe is the right to create more instances
/// of it and so to answer other users' requests. Who may do what is checked per request.
const PIPE_SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x12008b;;;AU)";

// Access rights that change a file or folder (write data / add file, append data / add folder,
// delete child, delete, change permissions, take ownership, generic write and all).
const WRITE_DATA: u32 = 0x2;
const APPEND_DATA: u32 = 0x4;
const DELETE_CHILD: u32 = 0x40;
const DELETE: u32 = 0x1_0000;
const WRITE_DAC: u32 = 0x4_0000;
const WRITE_OWNER: u32 = 0x8_0000;
const GENERIC_ALL: u32 = 0x1000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
/// Change a file, or what a folder contains.
pub(super) const CHANGE: u32 = WRITE_DATA | APPEND_DATA | DELETE_CHILD | DELETE | WRITE_DAC | WRITE_OWNER | GENERIC_ALL | GENERIC_WRITE;
/// Replace what a folder contains (rename or delete what is in it), not add to it.
pub(super) const REPLACE: u32 = DELETE_CHILD | DELETE | WRITE_DAC | WRITE_OWNER | GENERIC_ALL;

define_windows_service!(ffi_main, service_main);
static STATUS: OnceLock<ServiceStatusHandle> = OnceLock::new();

pub fn serve() -> Result<()> {
    // Started by the service control manager: run as a service. From a console: in the foreground.
    match service_dispatcher::start(NAME, ffi_main) {
        Ok(()) => Ok(()),
        Err(windows_service::Error::Winapi(e)) if e.raw_os_error() == Some(ERROR_FAILED_SERVICE_CONTROLLER_CONNECT as i32) => listen(),
        Err(e) => Err(e.into()),
    }
}

fn status(state: ServiceState, exit: u32) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: if state == ServiceState::Running {
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
        } else {
            ServiceControlAccept::empty()
        },
        exit_code: ServiceExitCode::Win32(exit),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    }
}

fn service_main(_: Vec<OsString>) {
    let handler = service_control_handler::register(NAME, |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            // Answer the request first; the pipe loop never returns, so end the process shortly after.
            if let Some(h) = STATUS.get() {
                let _ = h.set_service_status(status(ServiceState::StopPending, 0));
            }
            std::thread::spawn(|| {
                std::thread::sleep(Duration::from_millis(200));
                if let Some(h) = STATUS.get() {
                    let _ = h.set_service_status(status(ServiceState::Stopped, 0));
                }
                std::process::exit(0);
            });
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    });
    let Ok(handle) = handler else { return };
    let _ = STATUS.set(handle);
    log_to_file();
    let _ = handle.set_service_status(status(ServiceState::Running, 0));
    if let Err(err) = listen() {
        eprintln!("furnish service: {err:#}");
        let _ = handle.set_service_status(status(ServiceState::Stopped, 1));
    }
}

/// A service has no console: its messages (e.g. why a preset collection couldn't be set up) go
/// to service.log in the system directory, which administrators can read.
fn log_to_file() {
    use std::os::windows::io::IntoRawHandle;
    let path = crate::catalog::system_dir().join("service.log");
    // Above 1 MiB the old log becomes service.log.1, replacing the one before.
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 1 << 20) {
        let _ = std::fs::rename(&path, path.with_extension("log.1"));
    }
    let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) else { return };
    // Only administrators may read it: it has details of failures that users don't see.
    if let Ok(sd) = security_descriptor(LOG_SDDL) {
        let w = wide(&path.to_string_lossy());
        // SAFETY: sd is a valid descriptor; its DACL pointer points into it.
        unsafe {
            let (mut present, mut dacl, mut defaulted) = (0, null_mut::<ACL>(), 0);
            if GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted) != 0 {
                let info = DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION;
                SetNamedSecurityInfoW(w.as_ptr(), SE_FILE_OBJECT, info, null_mut(), null_mut(), dacl, null());
            }
            LocalFree(sd as _);
        }
    }
    // SAFETY: the handle stays open for the life of the process (into_raw_handle leaks it).
    unsafe { windows_sys::Win32::System::Console::SetStdHandle(windows_sys::Win32::System::Console::STD_ERROR_HANDLE, file.into_raw_handle()) };
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn last_error() -> std::io::Error {
    std::io::Error::last_os_error()
}

/// A security descriptor from SDDL; lives as long as the service.
fn security_descriptor(sddl: &str) -> Result<PSECURITY_DESCRIPTOR> {
    let mut sd: PSECURITY_DESCRIPTOR = null_mut();
    // SAFETY: valid NUL-terminated SDDL; sd receives a LocalAlloc'd descriptor we keep.
    if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(wide(sddl).as_ptr(), SDDL_REVISION_1, &mut sd, null_mut()) } == 0 {
        bail!("invalid security descriptor: {}", last_error());
    }
    Ok(sd)
}

fn listen() -> Result<()> {
    super::start();
    let name = wide(&super::socket_path().to_string_lossy());
    let sd = security_descriptor(PIPE_SDDL)?;
    let mut first = true;
    eprintln!("furnish service listening on {}", super::socket_path().display());
    loop {
        let sa = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd, bInheritHandle: 0 };
        // The first instance must be ours: nobody else may have created the pipe before us.
        // Overlapped, so that reads and writes can time out.
        let flags = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { 0 };
        let mode = PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;
        // SAFETY: valid name and security attributes.
        let handle = unsafe { CreateNamedPipeW(name.as_ptr(), flags, mode, PIPE_UNLIMITED_INSTANCES, 65536, 65536, 0, &sa) };
        if handle == INVALID_HANDLE_VALUE {
            bail!("creating the pipe: {}", last_error());
        }
        first = false;
        let pipe = Arc::new(Pipe { handle, read_timeout_ms: AtomicU32::new(millis(super::IO_TIMEOUT)) });
        if let Err(err) = pipe.connect() {
            eprintln!("warning: {err}");
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        let authorizing = pipe.clone();
        let input = super::Deadline {
            inner: PipeRef(pipe.clone()),
            until: Instant::now() + super::IO_TIMEOUT,
            set: |p, left| {
                p.0.set_read_timeout(left);
                Ok(())
            },
        };
        super::spawn_connection(input, PipeRef(pipe), finish, move |_| authorize(authorizing.handle));
    }
}

/// When the client has all answers: wait (not forever) until it has read them and closed the pipe.
fn finish(mut pipe: PipeRef) {
    use std::io::Read;
    let mut buf = [0u8; 256];
    let until = Instant::now() + super::IO_TIMEOUT;
    while let Some(left) = until.checked_duration_since(Instant::now()) {
        pipe.0.set_read_timeout(left);
        if !matches!(pipe.read(&mut buf), Ok(n) if n > 0) {
            break;
        }
    }
}

fn millis(d: Duration) -> u32 {
    d.as_millis().clamp(1, u32::MAX as u128 - 1) as u32
}

/// A connected instance of the service's pipe, with timeouts on reads and writes.
struct Pipe {
    handle: HANDLE,
    /// Of the next read; writes time out after `IO_TIMEOUT`.
    read_timeout_ms: AtomicU32,
}

// SAFETY: a pipe handle can be used from any thread; overlapped reads and writes may run at once.
unsafe impl Send for Pipe {}
unsafe impl Sync for Pipe {}

impl Drop for Pipe {
    fn drop(&mut self) {
        // SAFETY: handle is ours; no I/O is pending (every operation waits for its completion).
        unsafe {
            DisconnectNamedPipe(self.handle);
            CloseHandle(self.handle);
        }
    }
}

impl Pipe {
    fn set_read_timeout(&self, timeout: Duration) {
        self.read_timeout_ms.store(millis(timeout), Ordering::Relaxed);
    }

    /// Wait for a client.
    fn connect(&self) -> std::io::Result<()> {
        self.overlapped(u32::MAX, |ov| {
            // SAFETY: valid handle; ov lives until the operation completes.
            if unsafe { ConnectNamedPipe(self.handle, ov) } != 0 {
                return Ok(false);
            }
            let err = last_error();
            if err.raw_os_error() == Some(ERROR_PIPE_CONNECTED as i32) { Ok(false) } else { Err(err) }
        })
        .map(|_| ())
    }

    /// Start an operation (`start` returns whether it is pending), wait for it at most `timeout_ms`
    /// (then cancel it), return how many bytes it moved.
    fn overlapped(&self, timeout_ms: u32, start: impl FnOnce(*mut OVERLAPPED) -> std::io::Result<bool>) -> std::io::Result<usize> {
        // SAFETY: a manual-reset event for the operation, closed at the end.
        let event = unsafe { CreateEventW(null(), 1, 0, null()) };
        if event.is_null() {
            return Err(last_error());
        }
        // SAFETY: zeroed OVERLAPPED is valid; it outlives the operation, since we wait for it.
        let mut ov: OVERLAPPED = unsafe { std::mem::zeroed() };
        ov.hEvent = event;
        let result = (|| {
            match start(&mut ov) {
                Ok(true) => {}
                Ok(false) => return Ok(0),
                Err(err) if err.raw_os_error() == Some(ERROR_IO_PENDING as i32) => {}
                Err(err) => return Err(err),
            }
            // SAFETY: event and handle are valid; after CancelIoEx, GetOverlappedResult waits until
            // the operation is really over, so ov and the buffer stay valid as long as needed.
            unsafe {
                if WaitForSingleObject(event, timeout_ms) != WAIT_OBJECT_0 {
                    CancelIoEx(self.handle, &ov);
                }
                let mut n = 0u32;
                if GetOverlappedResult(self.handle, &ov, &mut n, 1) == 0 {
                    let err = last_error();
                    if err.raw_os_error() == Some(ERROR_OPERATION_ABORTED as i32) {
                        return Err(std::io::ErrorKind::TimedOut.into());
                    }
                    return Err(err);
                }
                Ok(n as usize)
            }
        })();
        // SAFETY: event came from CreateEventW.
        unsafe { CloseHandle(event) };
        result
    }

    fn io(&self, timeout_ms: u32, start: impl FnOnce(*mut OVERLAPPED) -> i32) -> std::io::Result<usize> {
        self.overlapped(timeout_ms, |ov| if start(ov) != 0 { Ok(true) } else { Err(last_error()) })
    }
}

/// A pipe shared by the threads of a connection.
struct PipeRef(Arc<Pipe>);

impl std::io::Read for PipeRef {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let len = buf.len().min(u32::MAX as usize) as u32;
        // SAFETY: buf is valid for len bytes until the operation completes (io waits for it).
        match self.0.io(self.0.read_timeout_ms.load(Ordering::Relaxed), |ov| unsafe { ReadFile(self.0.handle, buf.as_mut_ptr(), len, null_mut(), ov) }) {
            Err(err) if err.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) => Ok(0),
            other => other,
        }
    }
}

impl std::io::Write for PipeRef {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let len = buf.len().min(u32::MAX as usize) as u32;
        // SAFETY: buf is valid for len bytes until the operation completes (io waits for it).
        self.0.io(millis(super::IO_TIMEOUT), |ov| unsafe { WriteFile(self.0.handle, buf.as_ptr(), len, null_mut(), ov) })
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Client side: the program answering on the pipe must be the registered furnish service (which
/// only administrators can register, and which runs as SYSTEM). Anyone else could have created
/// the pipe while the service wasn't running.
pub(super) fn check_server(pipe: &File) -> std::io::Result<()> {
    let mut pid = 0u32;
    // SAFETY: pipe is an open pipe handle.
    if unsafe { GetNamedPipeServerProcessId(pipe.as_raw_handle() as HANDLE, &mut pid) } == 0 {
        return Err(last_error());
    }
    let service = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .and_then(|m| m.open_service(NAME, ServiceAccess::QUERY_STATUS))
        .and_then(|s| s.query_status());
    match service {
        Ok(status) if status.current_state == ServiceState::Running && status.process_id == Some(pid) => Ok(()),
        _ => Err(std::io::Error::other(format!(
            "the program answering on {} isn't the furnish service (process {pid})",
            super::socket_path().display()
        ))),
    }
}

fn authorize(pipe: HANDLE) -> Result<()> {
    // SAFETY: impersonate the client only to read its token, then go back to SYSTEM.
    let token = unsafe {
        if ImpersonateNamedPipeClient(pipe) == 0 {
            bail!("can't tell who is asking: {}", last_error());
        }
        let mut token: HANDLE = null_mut();
        let ok = OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut token);
        RevertToSelf();
        if ok == 0 {
            bail!("can't tell who is asking: {}", last_error());
        }
        token
    };
    let group = crate::policy::load().service.and_then(|s| s.allowed_group);
    // Elevated administrators and SYSTEM; a non-elevated admin token has Administrators deny-only.
    let allowed = [Some(SYSTEM.to_string()), Some(ADMINISTRATORS.to_string()), group]
        .into_iter()
        .flatten()
        .any(|who| sid(&who).is_some_and(|sid| member(token, &sid)));
    // SAFETY: token came from OpenThreadToken.
    unsafe { CloseHandle(token) };
    if !allowed {
        bail!(super::REFUSED);
    }
    Ok(())
}

/// A SID ("S-1-…") or account name, as a SID in a buffer.
fn sid(who: &str) -> Option<Vec<u8>> {
    let w = wide(who);
    if who.starts_with("S-1-") {
        let mut p: PSID = null_mut();
        // SAFETY: valid string; p is LocalAlloc'd, copied out and freed.
        unsafe {
            if ConvertStringSidToSidW(w.as_ptr(), &mut p) == 0 {
                return None;
            }
            let len = windows_sys::Win32::Security::GetLengthSid(p) as usize;
            let bytes = std::slice::from_raw_parts(p as *const u8, len).to_vec();
            LocalFree(p as _);
            return Some(bytes);
        }
    }
    let (mut sid_len, mut domain_len, mut kind) = (0u32, 0u32, 0);
    // SAFETY: the first call asks for buffer sizes, the second fills them.
    unsafe {
        LookupAccountNameW(null(), w.as_ptr(), null_mut(), &mut sid_len, null_mut(), &mut domain_len, &mut kind);
        let mut sid = vec![0u8; sid_len as usize];
        let mut domain = vec![0u16; domain_len as usize];
        let ok = LookupAccountNameW(null(), w.as_ptr(), sid.as_mut_ptr().cast(), &mut sid_len, domain.as_mut_ptr(), &mut domain_len, &mut kind);
        (ok != 0).then_some(sid)
    }
}

fn member(token: HANDLE, sid: &[u8]) -> bool {
    let mut is = 0;
    // SAFETY: token is an impersonation token, sid a valid SID buffer.
    unsafe { CheckTokenMembership(token, sid.as_ptr() as PSID, &mut is) != 0 && is != 0 }
}

/// The service's directory must be writable only by SYSTEM and administrators: it holds the
/// collections the service installs from. Created with that ACL; an existing one must have it,
/// since users can create folders in ProgramData and could have prepared one (or a link to one).
pub(super) fn secure_system_dir(dir: &Path) -> Result<()> {
    let w = wide(&dir.to_string_lossy());
    if std::fs::symlink_metadata(dir).is_err() {
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let sa = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: security_descriptor(DIR_SDDL)?,
            bInheritHandle: 0,
        };
        // SAFETY: valid path and security attributes.
        if unsafe { CreateDirectoryW(w.as_ptr(), &sa) } == 0 {
            bail!("creating {}: {}", dir.display(), last_error());
        }
        return Ok(());
    }
    let refuse = |why: &str| {
        anyhow::anyhow!("{} wasn't created by the furnish service ({why}); remove it and start the service again", dir.display())
    };
    // The folder itself, not what a link (junction, symbolic link) in its place points to.
    // SAFETY: valid path; the handle is closed by File.
    let handle = unsafe {
        CreateFileW(
            w.as_ptr(),
            READ_CONTROL,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        bail!("opening {}: {}", dir.display(), last_error());
    }
    // SAFETY: handle is valid and ours.
    let file = unsafe { <File as std::os::windows::io::FromRawHandle>::from_raw_handle(handle as _) };
    // SAFETY: zeroed BY_HANDLE_FILE_INFORMATION is a valid out-parameter.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut info) } == 0 {
        bail!("reading {}: {}", dir.display(), last_error());
    }
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(refuse("it is a link"));
    }
    if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
        return Err(refuse("it isn't a folder"));
    }
    let mut sd: PSECURITY_DESCRIPTOR = null_mut();
    // SAFETY: the descriptor is LocalAlloc'd and freed below.
    let rc = unsafe {
        GetSecurityInfo(
            file.as_raw_handle() as HANDLE,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            &mut sd,
        )
    };
    if rc != 0 {
        bail!("reading the permissions of {}: {}", dir.display(), std::io::Error::from_raw_os_error(rc as i32));
    }
    let (mut control, mut revision) = (0u16, 0u32);
    // SAFETY: sd is a valid descriptor until LocalFree.
    let protected = unsafe { GetSecurityDescriptorControl(sd, &mut control, &mut revision) } != 0 && control & SE_DACL_PROTECTED != 0;
    // Also rights that only new files and folders in it inherit: the service creates those.
    let other = unsafe { untrusted(sd, CHANGE, true) };
    unsafe { LocalFree(sd as _) };
    if let Some(who) = other {
        return Err(refuse(&format!("{who} can change it")));
    }
    if !protected {
        return Err(refuse("it inherits permissions from its parent"));
    }
    Ok(())
}

/// Who besides SYSTEM, Administrators and TrustedInstaller can change `path` (with any of
/// `rights`), going by its owner and access list.
pub(super) fn changeable_by(path: &Path, rights: u32) -> Result<Option<String>> {
    let w = wide(&path.to_string_lossy());
    let mut sd: PSECURITY_DESCRIPTOR = null_mut();
    // SAFETY: the descriptor is LocalAlloc'd and freed below.
    let rc = unsafe {
        GetNamedSecurityInfoW(
            w.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            &mut sd,
        )
    };
    if rc != 0 {
        bail!("reading the permissions of {}: {}", path.display(), std::io::Error::from_raw_os_error(rc as i32));
    }
    // SAFETY: sd is valid until LocalFree.
    let who = unsafe { untrusted(sd, rights, false) };
    unsafe { LocalFree(sd as _) };
    Ok(who)
}

/// The owner, if it isn't SYSTEM, Administrators or TrustedInstaller, or the first other account
/// that the access list grants any of `rights` (`children`: also rights only inherited by new
/// files and folders in it).
///
/// # Safety
/// `sd` must be a valid security descriptor with owner and DACL.
unsafe fn untrusted(sd: PSECURITY_DESCRIPTOR, rights: u32, children: bool) -> Option<String> {
    const ALLOWED: u8 = 0;
    const ALLOWED_OBJECT: u8 = 5;
    const ALLOWED_CALLBACK: u8 = 9;
    const ALLOWED_CALLBACK_OBJECT: u8 = 11;
    let trusted: Vec<Vec<u8>> = [SYSTEM, ADMINISTRATORS, TRUSTED_INSTALLER].iter().filter_map(|s| sid(s)).collect();
    // SAFETY (all below): pointers come from the descriptor and its ACL, which the caller keeps valid.
    let is_trusted = |p: PSID| trusted.iter().any(|t| unsafe { EqualSid(p, t.as_ptr() as PSID) } != 0);
    let (mut owner, mut defaulted): (PSID, i32) = (null_mut(), 0);
    if unsafe { GetSecurityDescriptorOwner(sd, &mut owner, &mut defaulted) } == 0 || owner.is_null() {
        return Some("an unknown owner".into());
    }
    if !is_trusted(owner) {
        return Some(format!("its owner {}", sid_string(owner)));
    }
    let (mut present, mut dacl, mut defaulted) = (0, null_mut::<ACL>(), 0);
    if unsafe { GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted) } == 0 || present == 0 || dacl.is_null() {
        return Some("everyone (it has no access list)".into());
    }
    for i in 0..unsafe { (*dacl).AceCount } {
        let mut ace = null_mut();
        if unsafe { GetAce(dacl, i.into(), &mut ace) } == 0 {
            return Some("an unreadable access list entry".into());
        }
        let header = unsafe { *(ace as *const ACE_HEADER) };
        if header.AceFlags as u32 & INHERIT_ONLY_ACE != 0 && !children {
            continue;
        }
        match header.AceType {
            ALLOWED | ALLOWED_CALLBACK => {
                let mask = unsafe { (*(ace as *const ACCESS_ALLOWED_ACE)).Mask };
                let sid = unsafe { (ace as *const u8).add(std::mem::offset_of!(ACCESS_ALLOWED_ACE, SidStart)) } as PSID;
                if mask & rights != 0 && !is_trusted(sid) {
                    return Some(sid_string(sid));
                }
            }
            // Directory-service entries, not used on files: don't guess.
            ALLOWED_OBJECT | ALLOWED_CALLBACK_OBJECT => return Some("an object access entry".into()),
            _ => {} // denied, audit
        }
    }
    None
}

fn sid_string(sid: PSID) -> String {
    let mut s = null_mut();
    // SAFETY: sid is valid; s is LocalAlloc'd, copied out and freed.
    unsafe {
        if ConvertSidToStringSidW(sid, &mut s) == 0 {
            return "an unknown account".into();
        }
        let len = (0..).take_while(|&i| *s.add(i) != 0).count();
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(s, len));
        LocalFree(s as _);
        text
    }
}
