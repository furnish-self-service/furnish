//! The window's title and icon, and the macOS Dock icon, follow the branding: its name and logo,
//! else furnish's own. Windows shows the window icon in the title bar, taskbar and Alt+Tab. On
//! Linux, X11 taskbars show it; Wayland compositors and app menus take the icon from the installed
//! furnish.desktop, which stays furnish's. macOS windows have no icon; the Dock icon set here
//! lasts while furnish runs.

use std::{fs, path::PathBuf, sync::Mutex, time::SystemTime};

use furnish::catalog;
use tauri::{AppHandle, Manager};

/// Pixels per side: Windows scales it down from Alt+Tab's largest size to the title bar's 16; the
/// Dock magnifies icons, at 2x on Retina screens.
const SIZE: u32 = if cfg!(target_os = "macos") { 512 } else { 256 };

/// The name and logo (with when it changed) shown, so the icon is only made again when they change.
type Key = (String, Option<(PathBuf, Option<SystemTime>)>);

static SHOWN: Mutex<Option<Key>> = Mutex::new(None);

/// Show the branding's name and logo, if they changed since the last call. Cheap otherwise.
pub fn refresh(app: &AppHandle, name: &str) {
    let Some(window) = app.get_webview_window("main") else { return };
    let logo = catalog::branding_logo().map(|p| {
        let modified = fs::metadata(&p).and_then(|m| m.modified()).ok();
        (p, modified)
    });
    let key = (name.to_string(), logo);
    let mut shown = SHOWN.lock().unwrap_or_else(|e| e.into_inner());
    if shown.as_ref() == Some(&key) {
        return;
    }
    if let Err(err) = window.set_title(name) {
        eprintln!("warning: can't change the window title: {err:#}");
    }
    let rgba = key.1.as_ref().and_then(|(path, _)| {
        crate::tray_icon::render_file(path, SIZE)
            .inspect_err(|err| eprintln!("warning: window icon: can't show the branding logo {}: {err:#}", path.display()))
            .ok()
    });
    #[cfg(not(target_os = "macos"))]
    {
        let icon = match &rgba {
            Some(rgba) => Some(tauri::image::Image::new(rgba, SIZE, SIZE)),
            None => app.default_window_icon().cloned(),
        };
        if let Some(icon) = icon
            && let Err(err) = window.set_icon(icon)
        {
            eprintln!("warning: can't change the window icon: {err:#}");
        }
    }
    #[cfg(target_os = "macos")]
    dock::set(app, rgba);
    *shown = Some(key);
}

#[cfg(target_os = "macos")]
mod dock {
    use super::SIZE;

    /// The Dock icon from SIZE x SIZE RGBA; `None`: the app bundle's icon again.
    pub fn set(app: &tauri::AppHandle, rgba: Option<Vec<u8>>) {
        let png = rgba.and_then(|rgba| png(&rgba).inspect_err(|err| eprintln!("warning: can't make the Dock icon: {err:#}")).ok());
        let _ = app.run_on_main_thread(move || {
            use objc2::{AllocAnyThread, MainThreadMarker};
            use objc2_app_kit::{NSApplication, NSImage};
            use objc2_foundation::NSData;
            let Some(mtm) = MainThreadMarker::new() else { return };
            let image = png.and_then(|png| NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(&png)));
            // SAFETY: on the main thread; nil restores the app's own icon.
            unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(image.as_deref()) };
        });
    }

    fn png(rgba: &[u8]) -> Result<Vec<u8>, png::EncodingError> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, SIZE, SIZE);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(rgba)?;
        writer.finish()?;
        Ok(out)
    }
}
