//! Linux distributions can launch multiple applications through one Electron executable.
use std::{
    collections::HashSet,
    ffi::{OsStr, OsString},
    os::unix::ffi::OsStrExt,
    path::Path,
};

use sysinfo::{ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind};

fn shared_runtime(name: &OsStr) -> bool {
    name.as_bytes()
        .strip_prefix(b"electron")
        .is_some_and(|suffix| suffix.iter().all(u8::is_ascii_digit))
}

pub(super) fn running_apps(system: &mut System) -> HashSet<String> {
    let live_runtime = |p: &sysinfo::Process| {
        p.status() != ProcessStatus::Zombie && shared_runtime(super::basename(p))
    };
    let pids: Vec<_> = system
        .processes()
        .values()
        .filter(|p| live_runtime(p))
        .map(|p| p.pid())
        .collect();
    if pids.is_empty() {
        return HashSet::new();
    }
    // Only request arguments for these runtimes. Recheck the executable in case a PID exited or exec'd.
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&pids),
        true,
        ProcessRefreshKind::nothing()
            .without_tasks()
            .with_exe(UpdateKind::Always)
            .with_cmd(UpdateKind::Always),
    );
    pids.into_iter()
        .filter_map(|pid| {
            let process = system.process(pid).filter(|p| live_runtime(p))?;
            process_package(process.cmd(), process.exe(), process.name())
                .map(str::to_ascii_lowercase)
        })
        .collect()
}

fn process_package<'a>(argv: &'a [OsString], exe: Option<&Path>, name: &OsStr) -> Option<&'a str> {
    if let [title] = argv {
        // Chromium's SetProcessTitleFromCommandLine flattens argv into a single string
        // beginning with its real executable. Paths with spaces lose their boundaries.
        let title = title.as_bytes();
        let tail = if let Some(exe) = exe {
            title
                .strip_prefix(exe.as_os_str().as_bytes())?
                .strip_prefix(b" ")?
        } else {
            let end = title.iter().position(|b| *b == b' ')?;
            let path = Path::new(OsStr::from_bytes(&title[..end]));
            if !path.is_absolute() || path.file_name()? != name {
                return None;
            }
            &title[end + 1..]
        };
        return package_name(tail.split(|b| *b == b' '));
    }
    package_name(argv.iter().skip(1).map(|s| s.as_bytes()))
}

fn package_name<'a>(mut args: impl Iterator<Item = &'a [u8]>) -> Option<&'a str> {
    // Electron's default_app takes the first positional argument or --app=..., and consumes
    // --require operands separately. Never search the application's own arguments for a name.
    let operand = loop {
        let arg = args.next()?;
        if let Some(path) = arg.strip_prefix(b"--app=") {
            break path.split(|b| *b == b'=').next()?;
        }
        if !arg.starts_with(b"-") {
            break arg;
        }
        match arg {
            b"--require" | b"-r" => {
                args.next()?;
            }
            b"--version" | b"-v" | b"--test-type=webdriver" => return None,
            _ if arg.starts_with(b"--type=") => return None,
            // With a bare switch of unknown arity the next argument might be a profile or
            // log path. Only accept unambiguous switches before the launch operand.
            b"--"
            | b"--autostart"
            | b"--no-sandbox"
            | b"--disable-gpu"
            | b"--no-help"
            | b"--enable-logging"
            | b"--disable-dev-shm-usage"
            | b"--disable-setuid-sandbox"
            | b"--enable-wayland-ime" => {}
            _ if arg.starts_with(b"--") && arg.contains(&b'=') => {}
            _ => return None,
        }
    };
    let path = Path::new(OsStr::from_bytes(operand));
    if !path.is_absolute() || path.file_name()? != "app.asar" {
        return None;
    }
    let parent = path.parent()?;
    let app_dir = if parent.file_name()? == "resources" {
        parent.parent()?
    } else {
        parent
    };
    app_dir.file_name()?.to_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_only_shared_runtime_basenames() {
        for accepted in ["electron", "electron43"] {
            assert!(shared_runtime(OsStr::new(accepted)));
        }
        for other in [
            "Electron",
            "electronhelper",
            "electron43-helper",
            "my-electron",
            "electron.43",
        ] {
            assert!(!shared_runtime(OsStr::new(other)));
        }
    }

    #[test]
    fn reads_only_the_packaged_application_operand() {
        let app = |args: &[&str]| {
            process_package(
                &args.iter().map(OsString::from).collect::<Vec<_>>(),
                Some(Path::new("/usr/lib/electron43/electron")),
                OsStr::new("electron"),
            )
            .map(str::to_owned)
        };
        let expected = Some("Notes".to_string());
        assert_eq!(
            app(&[
                "/usr/lib/electron43/electron --user-data-dir=/lib/Other/app.asar --autostart /opt/Notes/resources/app.asar /lib/Other/app.asar"
            ]),
            expected
        );
        assert_eq!(
            app(&[
                "/usr/lib/electron43/electron /lib/Other/app.asar /opt/Notes/resources/app.asar"
            ]),
            Some("Other".into())
        );
        assert_eq!(
            app(&["/usr/lib/electron43/electron --type=renderer /opt/Notes/resources/app.asar"]),
            None
        );
        assert_eq!(
            app(&["/wrong/electron /opt/Notes/resources/app.asar"]),
            None
        );
        assert_eq!(
            app(&["/usr/lib/electron43/electronic /opt/Notes/resources/app.asar"]),
            None
        );
        assert_eq!(
            app(&[
                "wrapper",
                "--ozone-platform=wayland",
                "--autostart",
                "-r",
                "/lib/plugin/app.asar",
                "/opt/Notes/resources/app.asar"
            ]),
            expected
        );
        assert_eq!(
            app(&["anything", "--no-sandbox", "--app=/usr/lib/Notes/app.asar"]),
            expected
        );
        assert_eq!(
            app(&["electron", "/lib/Other/app.asar", "/lib/Notes/app.asar"]),
            Some("Other".into())
        );
        for args in [
            &["electron", "--type=renderer", "/lib/Notes/app.asar"][..],
            &["electron", "--user-data-dir", "/lib/Notes/app.asar"],
            &["electron", "--user-data-dir=/lib/Notes/app.asar"],
            &["electron", "--require", "/lib/Notes/app.asar"],
            &["electron", "--version", "/lib/Notes/app.asar"],
            &["electron", "./Notes/app.asar", "/lib/Notes/app.asar"],
            &["electron", "https://example.com/Notes/app.asar"],
            &["electron", "file:///lib/Notes/app.asar"],
            &[
                "electron",
                "/home/person/data.json",
                "--app=/lib/Notes/app.asar",
            ],
        ] {
            assert_eq!(app(args), None, "{args:?}");
        }
    }
}
