//! The tray icon (menu bar on macOS): open the window, check for updates, quit. Closing the window
//! keeps furnish running there, so automatic updates go on.

use std::{
    fs,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::SystemTime,
};

use furnish::catalog;

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
    pub settings: String,
    pub quit: String,
    /// Shown while a new version of furnish waits to be installed (see `selfupdate`).
    #[serde(default)]
    pub restart: String,
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
            settings: "Settings…".into(),
            quit: format!("Quit {brand}"),
            restart: "Restart to update".into(),
            status: String::new(),
            tooltip: brand.into(),
        }
    }
}

/// Installs, updates and uninstalls running now (see `Running`).
static RUNNING: AtomicUsize = AtomicUsize::new(0);
/// Quit was asked for while one ran: quit when it ends, start nothing new.
static QUITTING: AtomicBool = AtomicBool::new(false);

/// Installs, updates and uninstalls running now.
pub fn running() -> usize {
    RUNNING.load(Ordering::SeqCst)
}

/// A new version of furnish is downloaded: the menu offers Restart to update.
static UPDATE_READY: AtomicBool = AtomicBool::new(false);
/// The menu's texts as last sent, to show it again when that changes.
static TEXTS: Mutex<Option<Texts>> = Mutex::new(None);

pub fn set_update_ready(app: &AppHandle, ready: bool) {
    if UPDATE_READY.swap(ready, Ordering::SeqCst) != ready {
        let texts = TEXTS.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(texts) = texts {
            let _ = update(app, &texts);
        }
    }
}

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
    if UPDATE_READY.load(Ordering::SeqCst) && !texts.restart.is_empty() {
        menu.append(&MenuItem::with_id(app, "restart", &texts.restart, true, None::<&str>)?)?;
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
    menu.append(&MenuItem::with_id(
        app,
        "settings",
        &texts.settings,
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
            "settings" => {
                show(app);
                let _ = app.emit("furnish://open-settings", ());
            }
            "quit" => quit(app),
            // The window installs it once nothing is installing (see `selfupdate`).
            "restart" => {
                let _ = app.emit("furnish://restart-to-update", ());
            }
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
    let (key, icon) = wanted();
    builder.icon(image(icon.rgba)).icon_as_template(icon.template).build(app)?;
    *SHOWN.lock().unwrap_or_else(|e| e.into_inner()) = Some(key);
    watch_theme(app);
    Ok(())
}

/// The branding files (with when they changed) and the taskbar's darkness the icon is made from.
type Key = (Option<(PathBuf, Option<SystemTime>)>, Option<(PathBuf, Option<SystemTime>)>, bool);

/// What the icon shows now, so it is only made again when that changes.
static SHOWN: Mutex<Option<Key>> = Mutex::new(None);

fn key() -> Key {
    let stamp = |path: Option<PathBuf>| path.map(|p| (fs::metadata(&p).and_then(|m| m.modified()).ok(), p)).map(|(m, p)| (p, m));
    (stamp(catalog::branding_tray_icon()), stamp(catalog::branding_logo()), bar_is_dark())
}

fn wanted() -> (Key, crate::tray_icon::Icon) {
    let key = key();
    let path = |f: &Option<(PathBuf, Option<SystemTime>)>| f.as_ref().map(|(p, _)| p.clone());
    let icon = crate::tray_icon::choose(path(&key.0).as_deref(), path(&key.1).as_deref(), cfg!(target_os = "macos"), key.2);
    (key, icon)
}

fn image(rgba: Vec<u8>) -> tauri::image::Image<'static> {
    let size = crate::tray_icon::SIZE;
    tauri::image::Image::new_owned(rgba, size, size)
}

/// Show the icon for the current branding and taskbar theme, if either changed. Cheap otherwise.
pub fn refresh_icon(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(ID) else { return };
    let mut shown = SHOWN.lock().unwrap_or_else(|e| e.into_inner());
    if shown.as_ref() == Some(&key()) {
        return;
    }
    let (key, icon) = wanted();
    match tray.set_icon(Some(image(icon.rgba))).and_then(|()| tray.set_icon_as_template(icon.template)) {
        Ok(()) => *shown = Some(key),
        Err(err) => eprintln!("warning: can't change the tray icon: {err:#}"),
    }
}

/// Whether the taskbar (Windows) or panel (Linux) is dark, so one-color icons turn white.
#[cfg(windows)]
fn bar_is_dark() -> bool {
    crate::tray_icon::windows_bar_is_dark(windows_theme::system_uses_light_theme())
}

#[cfg(target_os = "linux")]
fn bar_is_dark() -> bool {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    crate::tray_icon::linux_bar_is_dark(&desktop, portal_color_scheme())
}

/// macOS colors template images itself.
#[cfg(target_os = "macos")]
fn bar_is_dark() -> bool {
    false
}

/// The freedesktop portal's `color-scheme`: 0 no preference, 1 dark, 2 light.
#[cfg(target_os = "linux")]
fn portal_color_scheme() -> Option<u32> {
    use gio::prelude::*;
    let bus = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>).ok()?;
    let read = |method: &str| {
        bus.call_sync(
            Some("org.freedesktop.portal.Desktop"),
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Settings",
            method,
            Some(&("org.freedesktop.appearance", "color-scheme").to_variant()),
            Some(glib::VariantTy::new("(v)").expect("valid type")),
            gio::DBusCallFlags::NONE,
            1000,
            None::<&gio::Cancellable>,
        )
        .ok()
    };
    // ReadOne since portal version 2; the older Read wraps the value in one more variant.
    let mut value = read("ReadOne").or_else(|| read("Read"))?.child_value(0);
    while let Some(inner) = value.as_variant() {
        value = inner;
    }
    value.get::<u32>()
}

/// Change the icon when the taskbar's theme changes (Windows: the registry value; Linux: the
/// portal's signal, delivered on the main loop since this runs on the main thread).
fn watch_theme(app: &AppHandle) {
    #[cfg(windows)]
    {
        // On the main thread, like every other refresh: none waits for another holding SHOWN.
        let app = app.clone();
        windows_theme::on_change(move || {
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || refresh_icon(&handle));
        });
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(bus) = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>) {
            let app = app.clone();
            bus.signal_subscribe(
                Some("org.freedesktop.portal.Desktop"),
                Some("org.freedesktop.portal.Settings"),
                Some("SettingChanged"),
                Some("/org/freedesktop/portal/desktop"),
                Some("org.freedesktop.appearance"),
                gio::DBusSignalFlags::NONE,
                move |_, _, _, _, _, _| refresh_icon(&app),
            );
        }
    }
    #[cfg(target_os = "macos")]
    let _ = app;
}

#[cfg(windows)]
mod windows_theme {
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, REG_NOTIFY_CHANGE_LAST_SET, RRF_RT_REG_DWORD, RegGetValueW,
        RegNotifyChangeKeyValue, RegOpenKeyExW,
    };

    /// The taskbar's theme; `AppsUseLightTheme` beside it is the apps' theme, which can differ.
    const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    pub fn system_uses_light_theme() -> Option<u32> {
        let (key, name) = (wide(KEY), wide("SystemUsesLightTheme"));
        let (mut value, mut size) = (0u32, 4u32);
        // SAFETY: NUL-terminated strings, and a 4-byte buffer with its size.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                (&raw mut value).cast(),
                &mut size,
            )
        };
        (status == 0).then_some(value)
    }

    /// Call `changed` after each change of a value under the key, from a thread of its own.
    pub fn on_change(changed: impl Fn() + Send + 'static) {
        std::thread::spawn(move || {
            let mut key: HKEY = std::ptr::null_mut();
            // SAFETY: a NUL-terminated string and a place for the handle, which this thread keeps.
            if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, wide(KEY).as_ptr(), 0, KEY_NOTIFY, &mut key) } != 0 {
                return;
            }
            // SAFETY: an open key; synchronous, so it returns once something changed.
            while unsafe { RegNotifyChangeKeyValue(key, 0, REG_NOTIFY_CHANGE_LAST_SET, std::ptr::null_mut(), 0) } == 0 {
                changed();
            }
        });
    }
}

/// New texts: the language, the brand or the status changed.
pub fn update(app: &AppHandle, texts: &Texts) -> tauri::Result<()> {
    *TEXTS.lock().unwrap_or_else(|e| e.into_inner()) = Some(texts.clone());
    if let Some(tray) = app.tray_by_id(ID) {
        tray.set_menu(Some(menu(app, texts)?))?;
        tray.set_tooltip(Some(&texts.tooltip))?;
    }
    // The branding may have changed with the policy or an added address.
    refresh_icon(app);
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
