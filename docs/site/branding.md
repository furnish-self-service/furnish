---
title: Brand the desktop app
order: 35
group: Administrators
previous: admins
---
# Brand the desktop app

## Branding

```json
{ "branding": { "name": "Acme Apps", "logo": "acme-it/icons/logo.svg", "accent": "#c2410c" } }
```

The app then shows your name, logo and color. furnish derives the lighter and darker shades of the color and keeps text in it readable in light and dark mode. `accentDark` sets the color for dark mode, and `theme.light` and `theme.dark` set any other color of the app.

![furnish as "Acme Apps" with an orange accent.](images/branded.webp)

The logo can be an SVG, PNG, JPEG or WebP file of up to 1 MB, given as a path inside one of your preset collections or as an absolute path. While furnish runs, the logo also replaces its icon on the window, in the taskbar and the Dock, and in the system tray and the macOS menu bar, in its own colors. On Linux, after furnish starts, the app menu and Wayland taskbar use the organization's name and logo from a per-user copy of the installed desktop entry. Removing the branding restores the package entry. furnish leaves an existing personal launcher customization in place. The Windows Start menu, pinned Windows taskbar buttons and macOS Launchpad keep furnish's icon. Text in an SVG logo doesn't show in these icons, so convert it to paths.

A tray icon in one color fits in better next to the other icons there:

```json
{ "branding": { "logo": "acme-it/icons/logo.svg", "trayIcon": "acme-it/icons/tray.svg" } }
```

Draw it in black on a transparent background, as a square or close to it. furnish shows it the way each system shows its own tray icons:

| System | The tray icon |
|---|---|
| Windows | black on a light taskbar, white on a dark one |
| macOS | as a template image: the menu bar colors it for light and dark mode |
| Linux | white, or black when the desktop uses light colors (on GNOME always white, since its top bar is dark) |

furnish uses `trayIcon` if it can read it, otherwise the logo, otherwise its own icon. It follows when the policy or the theme changes while it runs.
