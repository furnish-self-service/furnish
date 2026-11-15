//! Starting furnish at sign-in, hidden in the tray, as the `startAtSignIn` setting says: a Run
//! value in HKCU on Windows, a LaunchAgent on macOS, an XDG autostart entry on Linux.

/// The argument the sign-in entry passes: start with the window hidden.
pub const HIDDEN: &str = "--hidden";

/// The program the entry starts, as the entry's format wants it written. `os` is
/// `std::env::consts::OS`; `appimage` the AppImage furnish runs from, if any.
pub fn program(exe: &str, appimage: Option<&str>, os: &str) -> String {
    match os {
        // Unquoted, Windows would also try C:\Program.exe for C:\Program Files\….
        "windows" => format!("\"{exe}\""),
        "linux" => {
            // The AppImage itself, not the copy mounted while it runs. On Nix, the program from
            // PATH: the store path is gone after the next update.
            let path = match appimage {
                Some(image) => image,
                None if exe.starts_with("/nix/store/") => "furnish-gui",
                None => exe,
            };
            if path.contains(char::is_whitespace) {
                format!("\"{path}\"")
            } else {
                path.to_string()
            }
        }
        _ => exe.to_string(),
    }
}

fn launcher() -> anyhow::Result<auto_launch::AutoLaunch> {
    let exe = std::env::current_exe()?;
    let appimage = std::env::var("APPIMAGE").ok();
    let mut builder = auto_launch::AutoLaunchBuilder::new();
    builder
        .set_app_name(if cfg!(target_os = "macos") {
            "io.github.furnish-self-service.furnish.autostart"
        } else {
            "furnish"
        })
        .set_app_path(&program(
            &exe.to_string_lossy(),
            appimage.as_deref(),
            std::env::consts::OS,
        ))
        .set_args(&[HIDDEN])
        // macOS: System Settings lists the LaunchAgent under furnish's name and icon.
        .set_bundle_identifiers(&["io.github.furnish-self-service.furnish"])
        .set_windows_enable_mode(auto_launch::WindowsEnableMode::CurrentUser);
    Ok(builder.build()?)
}

/// Add or remove the sign-in entry. Written again each time, so it names the program furnish
/// runs from now (an AppImage may have moved). Development builds leave the system alone.
pub fn apply(on: bool) -> anyhow::Result<()> {
    if cfg!(debug_assertions) {
        return Ok(());
    }
    let launcher = launcher()?;
    let result = if on {
        launcher.enable()
    } else {
        launcher.disable()
    };
    match result {
        Err(auto_launch::Error::Io(e)) if !on && e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => Ok(result?),
    }
}

/// Apply the setting in effect (the policy may lock it); problems only go to the log.
pub fn sync() {
    if let Err(err) = apply(furnish::settings::load().start_at_sign_in) {
        eprintln!("warning: couldn't change starting at sign-in: {err:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::program;

    #[test]
    fn the_entry_starts_what_survives_updates() {
        assert_eq!(
            program(r"C:\Program Files\furnish\furnish.exe", None, "windows"),
            r#""C:\Program Files\furnish\furnish.exe""#
        );
        assert_eq!(
            program(
                "/Applications/furnish.app/Contents/MacOS/furnish",
                None,
                "macos"
            ),
            "/Applications/furnish.app/Contents/MacOS/furnish"
        );
        assert_eq!(
            program("/usr/bin/furnish-gui", None, "linux"),
            "/usr/bin/furnish-gui"
        );
        assert_eq!(
            program(
                "/tmp/.mount_x/usr/bin/furnish-gui",
                Some("/home/a/Apps/furnish.AppImage"),
                "linux"
            ),
            "/home/a/Apps/furnish.AppImage"
        );
        assert_eq!(
            program(
                "/tmp/.mount_x/usr/bin/furnish-gui",
                Some("/home/a/My Apps/furnish.AppImage"),
                "linux"
            ),
            "\"/home/a/My Apps/furnish.AppImage\""
        );
        assert_eq!(
            program(
                "/nix/store/abc-furnish-gui-0.1.0/bin/.furnish-gui-wrapped",
                None,
                "linux"
            ),
            "furnish-gui"
        );
    }
}
