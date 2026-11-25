//! What every file-writing resource shares: `~/` paths, links written through to their target,
//! text encodings, atomic writes that keep mode, owner and encoding, `backup`, `validate` and
//! `private`. One implementation for every system.

use std::{
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, anyhow, bail};

use super::closed_apps::ClosedApps;

/// A path as written in the app, `~/` expanded to the home folder (on Windows too).
pub fn expand(path: &str) -> Result<PathBuf> {
    let rest = if path == "~" {
        Some("")
    } else {
        path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\"))
    };
    let expanded = match rest {
        Some(rest) => in_home(rest)?,
        None => PathBuf::from(path),
    };
    if !expanded.is_absolute() {
        bail!("path '{path}' must be absolute or start with ~/");
    }
    if expanded.components().any(|c| c == Component::ParentDir) {
        bail!("path '{path}' must not contain ..");
    }
    Ok(expanded)
}

/// `rest` (separated by `/` or `\\`) in the home folder, with this system's separators.
fn in_home(rest: &str) -> Result<PathBuf> {
    let home = dirs::home_dir().context("no home folder to expand ~ in")?;
    Ok(rest.split(['/', '\\']).filter(|p| !p.is_empty()).fold(home, |p, part| p.join(part)))
}

/// A command argument as written in the app: a leading `~/` (or `~\\`), or a bare `~`, becomes the
/// home folder, as in paths. `~` anywhere else stays. `~~/…` and `~~` are the escape for a
/// literal `~/…` and `~`.
pub fn expand_arg(arg: &str) -> Result<String> {
    if arg == "~~" || arg.starts_with("~~/") || arg.starts_with("~~\\") {
        return Ok(arg[1..].to_string());
    }
    let rest = match arg.strip_prefix("~/").or_else(|| arg.strip_prefix("~\\")) {
        Some(rest) => rest,
        None if arg == "~" => "",
        None => return Ok(arg.to_string()),
    };
    let mut path = in_home(rest)?.to_string_lossy().into_owned();
    // A folder written with a trailing separator keeps it (`~/src/` for rsync and the like).
    if rest.ends_with(['/', '\\']) {
        path.push(std::path::MAIN_SEPARATOR);
    }
    Ok(path)
}

/// The file a path names: a symbolic link's target (also through several links), else the path.
pub fn resolve(path: &Path) -> Result<PathBuf> {
    let mut current = path.to_path_buf();
    for _ in 0..40 {
        match fs::symlink_metadata(&current) {
            Ok(m) if m.file_type().is_symlink() => {
                let target = fs::read_link(&current)?;
                current = match current.parent() {
                    Some(parent) if target.is_relative() => parent.join(target),
                    _ => target,
                };
            }
            _ => return Ok(current),
        }
    }
    bail!("{}: too many levels of symbolic links", path.display())
}

/// A file a resource manages: where it is as written (`shown`) and in fact (`real`), and its
/// contents (`None`: there is no file).
pub struct Managed {
    requires_closed: ClosedApps,
    pub shown: String,
    pub real: PathBuf,
    pub bytes: Option<Vec<u8>>,
    /// The file (or a folder on the way) exists but this account may not read it, e.g. a root-only file that the furnish
    /// service wrote, while furnish works out the app's status as the user.
    pub unreadable: bool,
}

impl Managed {
    pub fn open(path: &str) -> Result<Self> {
        Self::open_guarded(path, ClosedApps::default())
    }

    pub(super) fn open_guarded(path: &str, requires_closed: ClosedApps) -> Result<Self> {
        let real = resolve(&expand(path)?)?;
        let mut unreadable = false;
        let bytes = match fs::read(&real) {
            Ok(b) => Some(b),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) if real.is_dir() => bail!("{path} is a folder"),
            // Also when a folder on the way can't be opened: then furnish can't tell either.
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                unreadable = true;
                None
            }
            Err(e) => return Err(anyhow!(e).context(format!("reading {path}"))),
        };
        Ok(Managed { shown: path.to_string(), real, bytes, unreadable, requires_closed })
    }

    /// A settings app may have saved on exit, or restarted during validation. Only write
    /// the planned edit if its input still matches after checking the process list.
    pub(super) fn check_before_change(&self) -> Result<()> {
        if !self.requires_closed.is_empty() {
            self.requires_closed.ensure_closed()?;
            let now = Self::open(&self.shown)?;
            anyhow::ensure!(!now.unreadable && self.real == now.real && self.bytes == now.bytes,
                "{} changed while preparing this edit; retry to use the latest settings", self.shown);
        }
        Ok(())
    }

    /// The file's text (empty if there is no file), decoded from its encoding.
    pub fn text(&self) -> Result<Text> {
        match &self.bytes {
            Some(b) => Text::decode(b).with_context(|| format!("reading {}", self.shown)),
            None => Ok(Text::default()),
        }
    }

    /// The name for messages: the path as written, and where a link leads.
    pub fn name(&self) -> String {
        match expand(&self.shown) {
            Ok(p) if p != self.real => format!("{} (a link to {})", self.shown, self.real.display()),
            _ => self.shown.clone(),
        }
    }
}

/// The byte order mark a text file starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Bom {
    #[default]
    None,
    Utf8,
    Utf16Le,
    Utf16Be,
}

/// A text file's contents with `\n` line ends, and how to write them back as they were.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Text {
    pub text: String,
    pub bom: Bom,
    /// The file had Windows line ends (`\r\n`).
    pub crlf: bool,
}

impl Text {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let utf16 = |rest: &[u8], le: bool| {
            if !rest.len().is_multiple_of(2) {
                bail!("not UTF-16 text: odd length");
            }
            let units: Vec<u16> =
                rest.chunks(2).map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).collect();
            String::from_utf16(&units).map_err(|_| anyhow!("not UTF-16 text"))
        };
        let (bom, text) = if let Some(rest) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
            (Bom::Utf8, String::from_utf8(rest.to_vec()).map_err(|_| anyhow!("not UTF-8 text"))?)
        } else if let Some(rest) = bytes.strip_prefix(b"\xFF\xFE") {
            (Bom::Utf16Le, utf16(rest, true)?)
        } else if let Some(rest) = bytes.strip_prefix(b"\xFE\xFF") {
            (Bom::Utf16Be, utf16(rest, false)?)
        } else {
            (Bom::None, String::from_utf8(bytes.to_vec()).map_err(|_| anyhow!("not UTF-8 text"))?)
        };
        let crlf = text.contains("\r\n");
        let text = if crlf { text.replace("\r\n", "\n") } else { text };
        Ok(Text { text, bom, crlf })
    }

    /// `text` in this file's encoding and line ends.
    pub fn encode(&self, text: &str) -> Vec<u8> {
        let text = if self.crlf { text.replace("\r\n", "\n").replace('\n', "\r\n") } else { text.to_string() };
        match self.bom {
            Bom::None => text.into_bytes(),
            Bom::Utf8 => [b"\xEF\xBB\xBF".as_slice(), text.as_bytes()].concat(),
            Bom::Utf16Le => [0xFF, 0xFE].into_iter().chain(text.encode_utf16().flat_map(u16::to_le_bytes)).collect(),
            Bom::Utf16Be => [0xFE, 0xFF].into_iter().chain(text.encode_utf16().flat_map(u16::to_be_bytes)).collect(),
        }
    }
}

/// How a file is written, beyond its contents.
#[derive(Debug, Default, Clone)]
pub struct WriteOptions {
    /// Unix mode for the file; `None` keeps the current one (new files: the default).
    pub mode: Option<u32>,
    /// Only the user may read and write it (Unix: mode 600, Windows: an access list).
    pub private: bool,
    /// Keep a timestamped copy of the current file first.
    pub backup: bool,
    /// A command to run on the new file first, `%s` standing for its path.
    pub validate: Option<Vec<String>>,
}

/// Write `bytes` to the managed file: through a temporary file beside it that replaces it, so
/// that readers see the old or the new file and never a part. Creates missing parent folders.
pub fn write(file: &Managed, bytes: &[u8], options: &WriteOptions) -> Result<()> {
    let real = &file.real;
    let parent = real.parent().with_context(|| format!("{}: no parent folder", file.shown))?;
    let unwritable = |e: std::io::Error| {
        if file.name() != file.shown {
            anyhow!("can't write {}: its target isn't writable ({e})", file.name())
        } else {
            anyhow!("can't write {}: {e}", file.shown)
        }
    };
    file.check_before_change()?;
    fs::create_dir_all(parent).map_err(unwritable)?;
    let existing = fs::metadata(real).ok();
    if existing.as_ref().is_some_and(|m| m.permissions().readonly()) {
        return Err(unwritable(std::io::Error::from(std::io::ErrorKind::PermissionDenied)));
    }
    let temp = Temp::create(parent, real).map_err(unwritable)?;
    let mut f = fs::OpenOptions::new().write(true).open(&temp.0)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    if let Some(command) = &options.validate {
        validate(command, &temp.0, &file.shown)?;
    }
    file.check_before_change()?;
    if options.backup && file.bytes.is_some() {
        backup(real, options.private)?;
    }
    temp.replace(real, existing.as_ref(), options).map_err(unwritable)
}

/// Delete the managed file (after a backup if asked for).
pub fn delete(file: &Managed, options: &WriteOptions) -> Result<()> {
    if file.bytes.is_none() {
        return Ok(());
    }
    file.check_before_change()?;
    if options.backup {
        backup(&file.real, options.private)?;
    }
    fs::remove_file(&file.real).with_context(|| format!("deleting {}", file.name()))
}

/// A temporary file beside the target; removed unless it replaced the target.
struct Temp(PathBuf);

impl Temp {
    fn create(dir: &Path, target: &Path) -> std::io::Result<Self> {
        let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        for attempt in 0..100u32 {
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
            // The target's name last, so a validate command that looks at the extension sees it.
            let path = dir.join(format!(".furnish-{}-{nanos:x}{attempt}.{name}", std::process::id()));
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            match options.open(&path) {
                Ok(_) => return Ok(Temp(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(std::io::Error::other("no free name for a temporary file"))
    }

    /// Give the new file the old one's mode and owner (or what `options` asks for), then put it
    /// in the target's place.
    fn replace(self, target: &Path, existing: Option<&fs::Metadata>, options: &WriteOptions) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            let mode = match (options.mode, options.private, existing) {
                (Some(mode), _, _) => mode,
                (None, true, _) => 0o600,
                (None, false, Some(m)) => m.mode() & 0o7777,
                (None, false, None) => 0o666 & !umask(),
            };
            fs::set_permissions(&self.0, fs::Permissions::from_mode(mode))?;
            if let Some(m) = existing {
                let new = fs::metadata(&self.0)?;
                if (new.uid(), new.gid()) != (m.uid(), m.gid()) && std::os::unix::fs::chown(&self.0, Some(m.uid()), Some(m.gid())).is_err() {
                    // Only root can give a file to someone else: write in place, which keeps the owner.
                    fs::copy(&self.0, target)?;
                    fs::set_permissions(target, fs::Permissions::from_mode(mode))?;
                    return Ok(());
                }
            }
            fs::rename(&self.0, target)?;
        }
        #[cfg(windows)]
        {
            let _ = options.mode; // Unix only
            if options.private {
                windows::make_private(&self.0, false)?;
            }
            match existing {
                // Keeps the old file's access list, attributes and creation time.
                Some(_) => windows::replace_file(target, &self.0)?,
                None => fs::rename(&self.0, target)?,
            }
            if options.private && existing.is_some() {
                windows::make_private(target, false)?;
            }
        }
        std::mem::forget(self);
        Ok(())
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[cfg(unix)]
fn umask() -> u32 {
    // SAFETY: umask only swaps the process's mask; a resource runs single-threaded.
    unsafe {
        let old = libc::umask(0o022);
        libc::umask(old);
        old as u32
    }
}

/// Copy `path` to `<path>.<UTC date and time>~` beside it.
fn backup(path: &Path, private: bool) -> Result<PathBuf> {
    let millis = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
    let stamp = crate::reports::rfc3339(millis)[..19].replace(':', "-");
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut copy = path.with_file_name(format!("{name}.{stamp}~"));
    for n in 1.. {
        if fs::symlink_metadata(&copy).is_err() {
            break;
        }
        copy = path.with_file_name(format!("{name}.{stamp}.{n}~"));
    }
    fs::copy(path, &copy).with_context(|| format!("backing up {} to {}", path.display(), copy.display()))?;
    if private {
        set_private(&copy, false)?;
    }
    Ok(copy)
}

/// Run `command` with `%s` replaced by `file`; fail with its output if it exits with another code than 0.
fn validate(command: &[String], file: &Path, shown: &str) -> Result<()> {
    let path = file.to_string_lossy();
    let args: Vec<String> = command.iter().map(|a| a.replace("%s", &path)).collect();
    let output = run(&args).with_context(|| format!("validate for {shown}"))?;
    if !output.status.success() {
        bail!(
            "validate for {shown}: `{}` exited with code {}; {shown} wasn't changed{}",
            command.join(" "),
            output.status.code().unwrap_or(-1),
            output_text(&output)
        );
    }
    Ok(())
}

/// `: <stderr or stdout>` of a command, for an error message; empty if it printed nothing.
pub fn output_text(output: &std::process::Output) -> String {
    let text = if output.stderr.iter().any(|b| !b.is_ascii_whitespace()) { &output.stderr } else { &output.stdout };
    let text = String::from_utf8_lossy(text).trim().to_string();
    if text.is_empty() { text } else { format!(": {text}") }
}

/// A program the resource needs isn't installed: furnish shows the app as "Needs <program>".
#[derive(Debug)]
pub struct Missing(pub String);

impl std::fmt::Display for Missing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "needs {}", self.0)
    }
}

impl std::error::Error for Missing {}

/// User install locations missing from a Finder or desktop-menu launch, in the
/// same order as Furnish/Package's Unix find_program.
#[cfg(unix)]
pub(super) fn desktop_bin_dirs() -> impl Iterator<Item = PathBuf> {
    dirs::home_dir().filter(|home| home.is_absolute()).map(|home| home.join(".local/bin"))
        .into_iter().chain([PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/local/bin")])
}

/// Only user commands may acquire user-writable executable directories.
/// The resource child of the service inherits its already-filtered PATH.
#[cfg(unix)]
fn command_path(inherited: Option<std::ffi::OsString>) -> Option<std::ffi::OsString> {
    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } == 0 || crate::service::in_service() || std::env::var("FURNISH_SERVICE").as_deref() == Ok("1") {
        return inherited;
    }
    // A bare executable name must not depend on the resource's working directory.
    let mut dirs: Vec<_> = inherited.as_ref()
        .map(|p| std::env::split_paths(p).filter(|dir| dir.is_absolute()).collect())
        .unwrap_or_default();
    for dir in desktop_bin_dirs() {
        if !dirs.contains(&dir) && std::env::join_paths([&dir]).is_ok() {
            dirs.push(dir);
        }
    }
    std::env::join_paths(dirs).ok()
}

pub(crate) fn on_command_path(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH");
    #[cfg(unix)]
    let path = command_path(path);
    which::which_in(program, path, ".").ok()
}

/// Run a command (no shell) and collect its output. A program that isn't installed is [`Missing`].
pub fn run(args: &[String]) -> Result<std::process::Output> {
    let (program, rest) = args.split_first().context("empty command")?;
    let path = std::env::var_os("PATH");
    #[cfg(unix)]
    let path = command_path(path);
    let resolved = find_program(program, path.as_deref()).ok_or_else(|| Missing(program.clone()))?;
    let mut command = Command::new(resolved);
    // Scripts using /usr/bin/env and tools invoking helpers need the same search
    // path as the top-level command. Scope it to this child, outside DSC discovery.
    #[cfg(unix)]
    if let Some(path) = path {
        command.env("PATH", path);
    }
    Ok(command.args(rest).stdin(std::process::Stdio::null()).output()?)
}

/// The program as given if it has a path, else where it is on PATH. On Windows also with the
/// extensions in PATHEXT, so `bw` finds `bw.cmd`. A newly installed mise tool may only
/// be in the shim directory until the next login.
fn find_program(program: &str, path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let given = Path::new(program);
    if given.components().count() > 1 {
        return given.exists().then(|| given.to_path_buf());
    }
    #[cfg(unix)]
    {
        which::which_in(program, path, ".").ok()
            .or_else(|| which::which(super::mise::shims_dir()?.join(program)).ok())
    }
    #[cfg(windows)]
    {
        let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        let extensions: Vec<_> = pathext.split(';').filter(|e| !e.is_empty()).map(str::to_lowercase).chain(Some(String::new())).collect();
        let in_dir = |dir: PathBuf| {
            extensions.iter().map(|ext| dir.join(format!("{program}{ext}"))).find(|p| p.is_file())
        };
        path.and_then(|path| std::env::split_paths(path).find_map(&in_dir))
            .or_else(|| super::mise::shims_dir().and_then(in_dir))
    }
}

/// Make a file or folder accessible only to the user: mode 600 (folders 700) on Unix, an access
/// list that grants only the user full control and inherits nothing on Windows.
pub fn set_private(path: &Path, folder: bool) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(if folder { 0o700 } else { 0o600 }))?;
    }
    #[cfg(windows)]
    windows::make_private(path, folder)?;
    Ok(())
}

/// Whether only the user can access the file or folder (see [`set_private`]).
pub fn is_private(path: &Path, folder: bool) -> Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(path)?.permissions().mode() & 0o777;
        Ok(mode == if folder { 0o700 } else { 0o600 })
    }
    #[cfg(windows)]
    {
        let _ = folder;
        windows::is_private(path)
    }
}

/// Whether only the owner may read the file: a mode without read access for group and others
/// (e.g. 600), or on Windows an access list for the user only.
pub fn owner_only(path: &Path) -> bool {
    #[cfg(unix)]
    {
        mode(path).is_some_and(|m| m & 0o044 == 0)
    }
    #[cfg(windows)]
    {
        windows::is_private(path).unwrap_or(false)
    }
}

/// The Unix mode of a file or folder (Windows: `None`).
pub fn mode(path: &Path) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).ok().map(|m| m.permissions().mode() & 0o7777)
    }
    #[cfg(windows)]
    {
        let _ = path;
        None
    }
}

/// A mode as written in the app: octal digits, e.g. `600` or `0700`.
pub fn parse_mode(mode: &str) -> Result<u32> {
    if !(3..=4).contains(&mode.len()) {
        bail!("mode '{mode}': three or four octal digits, e.g. '600'");
    }
    u32::from_str_radix(mode, 8).map_err(|_| anyhow!("mode '{mode}': three or four octal digits, e.g. '600'"))
}

pub fn set_mode(path: &Path, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    #[cfg(windows)]
    let _ = (path, mode);
    Ok(())
}

#[cfg(windows)]
pub(crate) use windows::user_sid;

#[cfg(windows)]
mod windows {
    use std::{os::windows::ffi::OsStrExt, path::Path, ptr::null_mut};

    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, LocalFree},
        Security::{
            ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW, SDDL_REVISION_1,
                SE_FILE_OBJECT, SetNamedSecurityInfoW,
            },
            DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetSecurityDescriptorControl, GetSecurityDescriptorDacl, GetTokenInformation,
            PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED, TOKEN_QUERY, TOKEN_USER, TokenUser,
        },
        Storage::FileSystem::{REPLACEFILE_IGNORE_MERGE_ERRORS, ReplaceFileW},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    /// Replace `target` with `new`, keeping the target's access list and attributes.
    pub fn replace_file(target: &Path, new: &Path) -> std::io::Result<()> {
        // SAFETY: valid NUL-terminated paths.
        let ok = unsafe { ReplaceFileW(wide(target).as_ptr(), wide(new).as_ptr(), std::ptr::null(), REPLACEFILE_IGNORE_MERGE_ERRORS, null_mut(), null_mut()) };
        if ok == 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }
    }

    /// The user this process runs as, as a SID string such as `S-1-5-21-…`.
    pub(crate) fn user_sid() -> std::io::Result<String> {
        // SAFETY: the token handle is closed below; the buffer is large enough for TOKEN_USER
        // (GetTokenInformation reports the size it needs).
        unsafe {
            let mut token: HANDLE = null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return Err(std::io::Error::last_os_error());
            }
            let mut len = 0u32;
            GetTokenInformation(token, TokenUser, null_mut(), 0, &mut len);
            let mut buf = vec![0u64; (len as usize).div_ceil(8)];
            let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len);
            CloseHandle(token);
            if ok == 0 {
                return Err(std::io::Error::last_os_error());
            }
            let user = &*(buf.as_ptr() as *const TOKEN_USER);
            let mut s = null_mut();
            if ConvertSidToStringSidW(user.User.Sid, &mut s) == 0 {
                return Err(std::io::Error::last_os_error());
            }
            let n = (0..).take_while(|&i| *s.add(i) != 0).count();
            let sid = String::from_utf16_lossy(std::slice::from_raw_parts(s, n));
            LocalFree(s as _);
            Ok(sid)
        }
    }

    /// Only the user has access (full control), nothing inherited; folders pass it on to what
    /// is created in them.
    pub fn make_private(path: &Path, folder: bool) -> std::io::Result<()> {
        let flags = if folder { "OICI" } else { "" };
        let sddl: Vec<u16> = format!("D:P(A;{flags};FA;;;{})", user_sid()?).encode_utf16().chain(Some(0)).collect();
        // SAFETY: valid SDDL; the descriptor is LocalAlloc'd and freed below; the DACL points into it.
        unsafe {
            let mut sd: PSECURITY_DESCRIPTOR = null_mut();
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut sd, null_mut()) == 0 {
                return Err(std::io::Error::last_os_error());
            }
            let (mut present, mut dacl, mut defaulted) = (0, null_mut::<ACL>(), 0);
            GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted);
            let rc = SetNamedSecurityInfoW(
                wide(path).as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                dacl,
                null_mut(),
            );
            LocalFree(sd as _);
            if rc != 0 {
                return Err(std::io::Error::from_raw_os_error(rc as i32));
            }
        }
        Ok(())
    }

    /// Whether the access list inherits nothing and grants access to the user only.
    pub fn is_private(path: &Path) -> anyhow::Result<bool> {
        let me = user_sid()?;
        let my_sid: Vec<u16> = me.encode_utf16().chain(Some(0)).collect();
        // SAFETY: the descriptor is LocalAlloc'd and freed below; ACEs point into it.
        unsafe {
            let mut mine: PSID = null_mut();
            if windows_sys::Win32::Security::Authorization::ConvertStringSidToSidW(my_sid.as_ptr(), &mut mine) == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            let mut sd: PSECURITY_DESCRIPTOR = null_mut();
            let mut dacl = null_mut::<ACL>();
            let rc = GetNamedSecurityInfoW(wide(path).as_ptr(), SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, null_mut(), null_mut(), &mut dacl, null_mut(), &mut sd);
            if rc != 0 {
                LocalFree(mine as _);
                return Err(std::io::Error::from_raw_os_error(rc as i32).into());
            }
            let (mut control, mut revision) = (0u16, 0u32);
            let mut private = GetSecurityDescriptorControl(sd, &mut control, &mut revision) != 0 && control & SE_DACL_PROTECTED != 0 && !dacl.is_null();
            if private {
                for i in 0..(*dacl).AceCount {
                    let mut ace = null_mut();
                    if GetAce(dacl, i.into(), &mut ace) == 0 {
                        private = false;
                        break;
                    }
                    let header = *(ace as *const ACE_HEADER);
                    if header.AceType != 0 {
                        continue; // not an allow entry
                    }
                    let sid = (ace as *const u8).add(std::mem::offset_of!(ACCESS_ALLOWED_ACE, SidStart)) as PSID;
                    if EqualSid(sid, mine) == 0 {
                        private = false;
                        break;
                    }
                }
            }
            LocalFree(sd as _);
            LocalFree(mine as _);
            Ok(private)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_paths_expand_on_every_system() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(expand("~/.ssh/config").unwrap(), home.join(".ssh").join("config"));
        assert_eq!(expand("~\\.ssh\\config").unwrap(), home.join(".ssh").join("config"));
        assert!(expand("relative/file").is_err());
        assert!(expand("~/../etc/passwd").is_err());
    }

    #[test]
    fn command_arguments_expand_a_leading_tilde_only() {
        let home = dirs::home_dir().unwrap();
        let shown = |p: PathBuf| p.to_string_lossy().into_owned();
        assert_eq!(expand_arg("~/.config/acme/acme.ovpn").unwrap(), shown(home.join(".config").join("acme").join("acme.ovpn")));
        assert_eq!(expand_arg("~\\acme.ovpn").unwrap(), shown(home.join("acme.ovpn")));
        assert_eq!(expand_arg("~").unwrap(), shown(home.clone()));
        assert_eq!(expand_arg("~/src/").unwrap(), format!("{}{}", shown(home.join("src")), std::path::MAIN_SEPARATOR));
        for literal in ["--config=~/x", "a~/b", "~user/x", "~x", "relative/~/x", "", "-"] {
            assert_eq!(expand_arg(literal).unwrap(), literal);
        }
        assert_eq!(expand_arg("~~/x").unwrap(), "~/x");
        assert_eq!(expand_arg("~~").unwrap(), "~");
        assert_eq!(expand_arg("~~x").unwrap(), "~~x");
    }

    #[test]
    fn a_bom_and_line_ends_are_kept() {
        let bytes = b"\xEF\xBB\xBFHost a\r\n  User b\r\n";
        let text = Text::decode(bytes).unwrap();
        assert_eq!((text.bom, text.crlf, text.text.as_str()), (Bom::Utf8, true, "Host a\n  User b\n"));
        assert_eq!(text.encode(&text.text), bytes);
        assert_eq!(text.encode("Host ä\n"), "\u{feff}Host ä\r\n".as_bytes());

        let utf16: Vec<u8> = [0xFF, 0xFE].into_iter().chain("Grüße\n".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        let text = Text::decode(&utf16).unwrap();
        assert_eq!((text.bom, text.text.as_str()), (Bom::Utf16Le, "Grüße\n"));
        assert_eq!(text.encode("Grüße\n"), utf16);
    }

    #[cfg(unix)]
    #[test]
    fn a_guarded_write_preserves_a_save_during_validation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        fs::write(&path, "old").unwrap();
        let guard = serde_json::from_value(serde_json::json!(["furnish-test-no-running-writer"])).unwrap();
        let file = Managed::open_guarded(path.to_str().unwrap(), guard).unwrap();
        let opts = WriteOptions {
            backup: true,
            validate: Some(vec!["sh".into(), "-c".into(), r#"printf saved > "$1""#.into(), "validator".into(), file.shown.clone(), "%s".into()]),
            ..Default::default()
        };
        assert!(write(&file, b"new", &opts).unwrap_err().to_string().contains("retry"));
        assert_eq!(fs::read(&path).unwrap(), b"saved");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn a_failing_validate_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        fs::write(&path, "old\n").unwrap();
        let file = Managed::open(path.to_str().unwrap()).unwrap();
        let fails = WriteOptions { validate: Some(vec![if cfg!(windows) { "cmd" } else { "false" }.into(), "%s".into()]), ..Default::default() };
        let fails = if cfg!(windows) { WriteOptions { validate: Some(vec!["cmd".into(), "/c".into(), "exit 1 %s".into()]), ..fails } } else { fails };
        let err = write(&file, b"new\n", &fails).unwrap_err();
        assert!(format!("{err:#}").contains("wasn't changed"), "{err:#}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "old\n");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1, "the temporary file is left");
    }

    #[test]
    fn backup_keeps_a_copy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, "{}").unwrap();
        let file = Managed::open(path.to_str().unwrap()).unwrap();
        write(&file, b"{\"a\": 1}", &WriteOptions { backup: true, ..Default::default() }).unwrap();
        let copies: Vec<_> = fs::read_dir(dir.path()).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with('~')).collect();
        assert_eq!(copies.len(), 1);
        assert_eq!(fs::read_to_string(copies[0].path()).unwrap(), "{}");
    }

    #[cfg(unix)]
    #[test]
    fn writing_keeps_the_mode_and_creates_parents() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("glab-cli").join("config.yml");
        let file = Managed::open(path.to_str().unwrap()).unwrap();
        write(&file, b"a: 1\n", &WriteOptions::default()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let file = Managed::open(path.to_str().unwrap()).unwrap();
        write(&file, b"a: 2\n", &WriteOptions::default()).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_is_written_through_and_a_read_only_target_fails_naming_the_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        fs::write(&target, "old").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink("target", &link).unwrap();
        let file = Managed::open(link.to_str().unwrap()).unwrap();
        write(&file, b"new", &WriteOptions::default()).unwrap();
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "the link was replaced");
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");

        // Like a file Home Manager links from the Nix store: a read-only file in a read-only folder.
        let store = dir.path().join("store");
        fs::create_dir(&store).unwrap();
        fs::write(store.join("config"), "managed").unwrap();
        fs::set_permissions(store.join("config"), fs::Permissions::from_mode(0o444)).unwrap();
        fs::set_permissions(&store, fs::Permissions::from_mode(0o555)).unwrap();
        let link = dir.path().join("config");
        std::os::unix::fs::symlink(store.join("config"), &link).unwrap();
        let file = Managed::open(link.to_str().unwrap()).unwrap();
        let err = format!("{:#}", write(&file, b"new", &WriteOptions::default()).unwrap_err());
        fs::set_permissions(&store, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(err.contains(link.to_str().unwrap()) && err.contains("isn't writable"), "{err}");
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(store.join("config")).unwrap(), "managed");
    }
}
