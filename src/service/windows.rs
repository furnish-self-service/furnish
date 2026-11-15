//! The furnish service on Windows: the Windows service `furnish`, running as SYSTEM, answers on
//! the named pipe `\\.\pipe\furnish`. Callers are identified by impersonating them.

use std::{
    ffi::OsString,
    fs::File,
    os::windows::io::{AsRawHandle, FromRawHandle},
    path::Path,
    ptr::{null, null_mut},
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

use anyhow::{Result, bail};
use windows_service::{
    define_windows_service,
    service::{ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType},
    service_control_handler::{self, ServiceControlHandlerResult, ServiceStatusHandle},
    service_dispatcher,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_FAILED_SERVICE_CONTROLLER_CONNECT, ERROR_PIPE_CONNECTED, GetLastError, HANDLE, INVALID_HANDLE_VALUE, LocalFree},
    Security::{
        Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, ConvertStringSidToSidW, GetNamedSecurityInfoW, SDDL_REVISION_1,
            SE_FILE_OBJECT,
        },
        CheckTokenMembership, DACL_SECURITY_INFORMATION, EqualSid, GetSecurityDescriptorControl, LookupAccountNameW,
        OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, RevertToSelf, SE_DACL_PROTECTED, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    },
    Storage::FileSystem::{CreateDirectoryW, FILE_FLAG_FIRST_PIPE_INSTANCE, FlushFileBuffers, PIPE_ACCESS_DUPLEX},
    System::{
        Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, ImpersonateNamedPipeClient, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
        },
        Threading::{GetCurrentThread, OpenThreadToken},
    },
};

use super::Out;

const NAME: &str = "furnish";
const SYSTEM: &str = "S-1-5-18";
const ADMINISTRATORS: &str = "S-1-5-32-544";
/// SYSTEM and Administrators have full control, users may read; protected from inheriting
/// ProgramData's "users may create files".
const DIR_SDDL: &str = "O:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FRFX;;;BU)";
/// Authenticated users may connect; who may do what is checked per request.
const PIPE_SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;AU)";

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
    // ponytail: appends forever; rotate if it grows (it gets a few lines per start and problem).
    let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) else { return };
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
        let flags = PIPE_ACCESS_DUPLEX | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { 0 };
        let mode = PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;
        // SAFETY: valid name and security attributes.
        let pipe = unsafe { CreateNamedPipeW(name.as_ptr(), flags, mode, PIPE_UNLIMITED_INSTANCES, 65536, 65536, 0, &sa) };
        if pipe == INVALID_HANDLE_VALUE {
            bail!("creating the pipe: {}", last_error());
        }
        first = false;
        // SAFETY: pipe is a valid handle; File takes ownership and closes it.
        let connected = unsafe { ConnectNamedPipe(pipe, null_mut()) } != 0 || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
        let file = unsafe { File::from_raw_handle(pipe as _) };
        if !connected {
            continue;
        }
        std::thread::spawn(move || {
            let raw = file.as_raw_handle() as HANDLE;
            let out: Out = match file.try_clone() {
                Ok(f) => Arc::new(Mutex::new(Box::new(f))),
                Err(err) => return eprintln!("warning: {err}"),
            };
            super::handle(&file, out, &|| authorize(raw));
            // Let the client read everything before the pipe closes.
            // SAFETY: raw belongs to `file`, still open.
            unsafe { FlushFileBuffers(raw) };
        });
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
/// since users can create folders in ProgramData and could have prepared one.
pub(super) fn secure_system_dir(dir: &Path) -> Result<()> {
    let w = wide(&dir.to_string_lossy());
    if !dir.exists() {
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
    let (mut owner, mut sd): (PSID, PSECURITY_DESCRIPTOR) = (null_mut(), null_mut());
    // SAFETY: owner points into sd, which we free at the end.
    unsafe {
        let rc = GetNamedSecurityInfoW(
            w.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            null_mut(),
            null_mut(),
            &mut sd,
        );
        if rc != 0 {
            bail!("reading the permissions of {}: {}", dir.display(), std::io::Error::from_raw_os_error(rc as i32));
        }
        let trusted_owner = [SYSTEM, ADMINISTRATORS].iter().filter_map(|s| sid(s)).any(|s| EqualSid(owner, s.as_ptr() as PSID) != 0);
        let (mut control, mut revision) = (0u16, 0u32);
        let protected = GetSecurityDescriptorControl(sd, &mut control, &mut revision) != 0 && control & SE_DACL_PROTECTED != 0;
        LocalFree(sd as _);
        if !trusted_owner || !protected {
            bail!(
                "{} wasn't created by the furnish service (other users may be able to change it); remove it and start the service again",
                dir.display()
            );
        }
    }
    Ok(())
}
