//! Locate the user's mise and expose the shim directory to new Windows sessions.
use std::{
    env,
    ffi::{OsStr, OsString},
    io::ErrorKind,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use windows_sys::Win32::{
    System::Environment::ExpandEnvironmentStringsW,
    UI::WindowsAndMessaging::{
        HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
    },
};
use winreg::{
    RegKey,
    enums::{HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_EXPAND_SZ, REG_SZ, RegType},
    transaction::Transaction,
    types::{FromRegValue, ToRegValue},
};

/// SYSTEM, LocalService and NetworkService each have their own HKCU and must never manage
/// the requesting person's tools there. The shared backend also excludes service operations.
pub(super) fn find_mise() -> Option<PathBuf> {
    let sid = crate::resources::common::user_sid().ok()?;
    if matches!(sid.as_str(), "S-1-5-18" | "S-1-5-19" | "S-1-5-20") {
        return None;
    }
    let find = |path: &OsStr| {
        env::split_paths(path)
            .filter(|p| p.is_absolute())
            .map(|p| p.join("mise.exe"))
            .find(|p| p.is_file())
    };
    env::var_os("PATH")
        .and_then(|path| find(&path))
        .or_else(|| {
            // A preceding install can change the persistent user PATH while Furnish is running.
            let key = RegKey::predef(HKEY_CURRENT_USER)
                .open_subkey("Environment")
                .ok()?;
            let (raw, kind) = read_path(&key).ok()?;
            find(&expanded(&raw, &kind).ok()?)
        })
        .or_else(|| {
            let home = dirs::home_dir();
            [
                dirs::data_local_dir().map(|p| p.join(r"Microsoft\WinGet\Links\mise.exe")),
                home.as_ref().map(|p| p.join(r"scoop\shims\mise.exe")),
                home.map(|p| p.join(r".cargo\bin\mise.exe")),
            ]
            .into_iter()
            .flatten()
            .find(|p| p.is_file())
        })
}

fn read_path(key: &RegKey) -> Result<(OsString, RegType)> {
    match key.get_raw_value("Path") {
        Ok(raw) => {
            ensure!(
                matches!(raw.vtype, REG_SZ | REG_EXPAND_SZ),
                "the user Path must be a registry string"
            );
            Ok((OsString::from_reg_value(&raw)?, raw.vtype))
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Ok((OsString::new(), REG_EXPAND_SZ)),
        Err(e) => Err(e).context("reading the user Path"),
    }
}

/// Compare expanded entries, but always append to the original registry string.
fn expanded(raw: &OsStr, kind: &RegType) -> Result<OsString> {
    if *kind != REG_EXPAND_SZ || !raw.as_encoded_bytes().contains(&b'%') {
        return Ok(raw.to_owned());
    }
    let source: Vec<_> = raw.encode_wide().chain(Some(0)).collect();
    let mut value = vec![0u16; source.len()];
    loop {
        // SAFETY: source is NUL terminated and value is writable through its declared length.
        let len = unsafe {
            ExpandEnvironmentStringsW(source.as_ptr(), value.as_mut_ptr(), value.len() as u32)
        } as usize;
        if len == 0 {
            return Err(std::io::Error::last_os_error()).context("expanding the user Path");
        }
        if len <= value.len() {
            return Ok(OsString::from_wide(&value[..len - 1]));
        }
        value.resize(len, 0);
    }
}

fn contains(path: &OsStr, dir: &Path) -> bool {
    env::split_paths(path).any(|entry| {
        entry
            .iter()
            .map(|c| c.to_ascii_lowercase())
            .eq(dir.iter().map(|c| c.to_ascii_lowercase()))
    })
}

pub(super) fn shims_on_path(shims: &Path) -> Result<bool> {
    let key = match RegKey::predef(HKEY_CURRENT_USER).open_subkey("Environment") {
        Ok(key) => key,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e).context("opening the user's environment"),
    };
    let (raw, kind) = read_path(&key)?;
    Ok(contains(&expanded(&raw, &kind)?, shims))
}

fn add_to_path(key: &RegKey, dir: &Path) -> Result<bool> {
    ensure!(
        dir.is_absolute(),
        "mise's shim directory must be an absolute path"
    );
    let (mut raw, kind) = read_path(key)?;
    if contains(&expanded(&raw, &kind)?, dir) {
        return Ok(false);
    }
    if !raw.is_empty() && !raw.as_encoded_bytes().ends_with(b";") {
        raw.push(";");
    }
    raw.push(env::join_paths([dir])?);
    ensure!(
        raw.encode_wide().count() < 32767,
        "the user Path is too long to add mise's shim directory"
    );
    ensure!(
        contains(&expanded(&raw, &kind)?, dir),
        "the expanded user Path cannot represent mise's shim directory"
    );
    let mut value = raw.to_reg_value();
    value.vtype = kind;
    key.set_raw_value("Path", &value)?;
    Ok(true)
}

fn persist_path(subkey: &str, shims: &Path) -> Result<bool> {
    // Keep other changes to the user's environment if installers run at the same time.
    let transaction = Transaction::new().context("starting the user Path update")?;
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey_transacted_with_flags(
        subkey,
        &transaction,
        KEY_QUERY_VALUE | KEY_SET_VALUE,
    )?;
    let changed = add_to_path(&key, shims)?;
    drop(key);
    transaction
        .commit()
        .context("saving the user Path for mise shims")?;
    Ok(changed)
}

pub(super) fn expose_shims(shims: &Path) -> Result<()> {
    if persist_path("Environment", shims)? {
        let environment: Vec<u16> = "Environment".encode_utf16().chain(Some(0)).collect();
        // SAFETY: the message uses a NUL-terminated string that lives throughout this call.
        // Notify Explorer so programs launched from the desktop inherit the new PATH.
        unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                0,
                environment.as_ptr() as isize,
                SMTO_ABORTIFHUNG,
                1000,
                std::ptr::null_mut(),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, process::Command};

    #[test]
    fn persisted_user_path_preserves_expansion_and_exposes_shims_to_a_new_shell() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("mise test shims");
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("furnish-test-mise-tool.cmd"),
            "@echo mise-shim-ran\r\n",
        )
        .unwrap();
        // Use a private registry key. This test leaves the real user's Path alone.
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let sandbox = format!(
            r"Software\furnish-tests\{}",
            temp.path().file_name().unwrap().to_string_lossy()
        );
        let (key, _) = hkcu.create_subkey(&sandbox).unwrap();
        let original = r"%SystemRoot%\System32;C:\another tool".to_owned();
        let mut value = original.to_reg_value();
        value.vtype = REG_EXPAND_SZ;
        key.set_raw_value("Path", &value).unwrap();
        let system = PathBuf::from(env::var_os("SystemRoot").unwrap()).join("System32");
        assert!(
            !add_to_path(&key, &system).unwrap(),
            "an expandable PATH entry is already configured"
        );
        assert!(add_to_path(&key, &directory).unwrap());
        assert!(
            !add_to_path(&key, &directory).unwrap(),
            "repeated installs must not duplicate the shim directory"
        );
        let (raw, kind) = read_path(&key).unwrap();
        assert_eq!(kind, REG_EXPAND_SZ);
        assert_eq!(raw, format!("{original};{}", directory.display()).as_str());
        let out = Command::new(system.join("cmd.exe"))
            .current_dir(temp.path())
            .env("PATH", expanded(&raw, &kind).unwrap())
            .args(["/d", "/c", "furnish-test-mise-tool.cmd"])
            .output()
            .unwrap();
        drop(key);
        hkcu.delete_subkey_all(sandbox).unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "mise-shim-ran");
    }
    #[test]
    fn transaction_persists_path_for_an_uninitialized_user_environment() {
        let temp = tempfile::tempdir().unwrap();
        let shims = temp.path().join("shims");
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let sandbox = format!(
            r"Software\furnish-tests\{}",
            temp.path().file_name().unwrap().to_string_lossy()
        );
        assert!(persist_path(&sandbox, &shims).unwrap());
        assert!(!persist_path(&sandbox, &shims).unwrap());
        let key = hkcu.open_subkey(&sandbox).unwrap();
        let (raw, kind) = read_path(&key).unwrap();
        assert_eq!(kind, REG_EXPAND_SZ);
        assert!(contains(&raw, &shims));
        drop(key);
        hkcu.delete_subkey_all(sandbox).unwrap();
    }
}
