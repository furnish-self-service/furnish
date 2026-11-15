//! The tray icon (menu bar on macOS): open the window, check for updates, quit. Closing the window
//! keeps furnish running there, so automatic updates go on.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use serde::Deserialize;
use tauri::{
    AppHandle, Emitter, Manager,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

const ID: &str = "furnish";

/// The menu's texts, in the app's language (the window sends them; see `set_tray`).
#[derive(Deserialize, Clone)]
pub struct Texts {
    pub open: String,
    pub check: String,
    pub quit: String,
    /// Updates waiting or what furnish is installing; empty: nothing.
    pub status: String,
    pub tooltip: String,
}

impl Texts {
    /// Until the window has loaded.
    pub fn english(brand: &str) -> Self {
        Texts {
            open: format!("Open {brand}"),
            check: "Check for updates".into(),
            quit: format!("Quit {brand}"),
            status: String::new(),
            tooltip: brand.into(),
        }
    }
}

/// Installs, updates and uninstalls running now (see `Running`).
static RUNNING: AtomicUsize = AtomicUsize::new(0);
/// Quit was asked for while one ran: quit when it ends, start nothing new.
static QUITTING: AtomicBool = AtomicBool::new(false);

/// Held while an install, update or uninstall runs. `None` once furnish is quitting.
pub struct Running(AppHandle);

impl Running {
    pub fn start(app: &AppHandle) -> Option<Self> {
        RUNNING.fetch_add(1, Ordering::SeqCst);
        if QUITTING.load(Ordering::SeqCst) {
            RUNNING.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(Running(app.clone()))
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if RUNNING.fetch_sub(1, Ordering::SeqCst) == 1 && QUITTING.load(Ordering::SeqCst) {
            self.0.exit(0);
        }
    }
}

/// Quit, but never in the middle of an install: then hide the window and quit after it.
pub fn quit(app: &AppHandle) {
    QUITTING.store(true, Ordering::SeqCst);
    if RUNNING.load(Ordering::SeqCst) == 0 {
        app.exit(0);
    } else if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

pub fn show(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        let _ = app.emit("furnish://shown", ());
    }
}

fn menu(app: &AppHandle, texts: &Texts) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    if !texts.status.is_empty() {
        menu.append(&MenuItem::with_id(
            app,
            "status",
            &texts.status,
            false,
            None::<&str>,
        )?)?;
        menu.append(&PredefinedMenuItem::separator(app)?)?;
    }
    menu.append(&MenuItem::with_id(
        app,
        "open",
        &texts.open,
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        "check",
        &texts.check,
        true,
        None::<&str>,
    )?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        "quit",
        &texts.quit,
        true,
        None::<&str>,
    )?)?;
    Ok(menu)
}

/// Add the tray icon. On Linux only with libayatana-appindicator (or libappindicator) installed:
/// without it, the tray library panics.
pub fn create(app: &AppHandle, texts: &Texts) -> tauri::Result<()> {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: loading the library runs no code of ours; the tray loads it the same way.
        let loads = |name: &str| unsafe { libloading::Library::new(name) }.is_ok();
        if !loads("libayatana-appindicator3.so.1") && !loads("libappindicator3.so.1") {
            eprintln!("note: no tray icon: libayatana-appindicator isn't installed");
            return Ok(());
        }
    }
    let builder = TrayIconBuilder::with_id(ID)
        .menu(&menu(app, texts)?)
        .tooltip(&texts.tooltip)
        // macOS shows the menu on click; on Windows (and Linux, where the desktop allows) a click opens the window.
        .show_menu_on_left_click(cfg!(target_os = "macos"))
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show(app),
            "check" => {
                let _ = app.emit("furnish://check-updates", ());
            }
            "quit" => quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
                && !cfg!(target_os = "macos")
            {
                show(tray.app_handle());
            }
        });
    // A monochrome template on macOS, which the menu bar tints for light and dark.
    #[cfg(target_os = "macos")]
    let builder = builder
        .icon(tauri::image::Image::from_bytes(include_bytes!(
            "../icons/tray-template.png"
        ))?)
        .icon_as_template(true);
    #[cfg(not(target_os = "macos"))]
    let builder = match app.default_window_icon() {
        Some(icon) => builder.icon(icon.clone()),
        None => builder,
    };
    builder.build(app)?;
    Ok(())
}

/// New texts: the language, the brand or the status changed.
pub fn update(app: &AppHandle, texts: &Texts) -> tauri::Result<()> {
    if let Some(tray) = app.tray_by_id(ID) {
        tray.set_menu(Some(menu(app, texts)?))?;
        tray.set_tooltip(Some(&texts.tooltip))?;
    }
    Ok(())
}

/// Whether people see the tray icon, so closing the window may keep furnish running.
pub fn shown(app: &AppHandle) -> bool {
    app.tray_by_id(ID).is_some() && desktop_shows_trays()
}

/// Linux: only with a StatusNotifierWatcher on the session bus. KDE Plasma, Xfce, Cinnamon and
/// others have one; GNOME only with the AppIndicator extension. Asked each time, since at
/// sign-in furnish may start before the panel.
#[cfg(target_os = "linux")]
fn desktop_shows_trays() -> bool {
    use gio::prelude::*;
    let Ok(bus) = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>) else {
        return false;
    };
    bus.call_sync(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "NameHasOwner",
        Some(&("org.kde.StatusNotifierWatcher",).to_variant()),
        Some(glib::VariantTy::new("(b)").expect("valid type")),
        gio::DBusCallFlags::NONE,
        1000,
        None::<&gio::Cancellable>,
    )
    .ok()
    .and_then(|reply| reply.get::<(bool,)>())
    .is_some_and(|(owned,)| owned)
}

#[cfg(not(target_os = "linux"))]
fn desktop_shows_trays() -> bool {
    true
}
