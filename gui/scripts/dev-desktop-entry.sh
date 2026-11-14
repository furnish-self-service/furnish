#!/bin/sh
# Give `tauri dev` builds furnish's icon on Linux desktops (Wayland finds window icons via an
# installed desktop file named like the app ID, "furnish"). Installed packages don't need this.
#
#   gui/scripts/dev-desktop-entry.sh           install ~/.local/share/applications/furnish.desktop
#   gui/scripts/dev-desktop-entry.sh --remove  remove it again
set -eu
apps="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
icons="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/256x256/apps"
if [ "${1:-}" = --remove ]; then
    rm -f "$apps/furnish.desktop" "$icons/furnish-gui.png"
else
    mkdir -p "$apps" "$icons"
    cp "$(dirname "$0")/../src-tauri/icons/128x128@2x.png" "$icons/furnish-gui.png"
    cat > "$apps/furnish.desktop" <<DESKTOP
[Desktop Entry]
Name=furnish (dev)
Exec=furnish-gui
Icon=furnish-gui
StartupWMClass=furnish
Terminal=false
Type=Application
NoDisplay=true
DESKTOP
fi
update-desktop-database "$apps" 2>/dev/null || true
