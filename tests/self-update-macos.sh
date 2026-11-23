#!/bin/bash
# furnish updates itself on macOS: the app from the .dmg (this build), in /Applications, takes the
# next version from a feed on this computer, first in the desktop app (automatic, hidden, it
# restarts into the new version), then by a person (Restart to update in the menu bar icon's menu,
# opened with a mouse click, with the window open), then with `furnish self-update`. Run on a
# throwaway Mac (CI): GitHub's runners let osascript use the accessibility API.
#
#   tests/self-update-macos.sh furnish_0.1.0_arm64.dmg <folder with latest.json, macos/…>
#
# The feed's folder comes from the update-feed job: the next version's .app.tar.gz, signed, and
# latest.json pointing at http://127.0.0.1:8765.
set -euo pipefail
dmg=${1:?usage: $0 <dmg> <feed folder>}; feed=${2:?usage: $0 <dmg> <feed folder>}
app=/Applications/furnish.app
next=$(jq -r .version "$feed/latest.json" | sed 's/-alpha\./+alpha./')
version() { furnish --version | sed 's/^furnish //'; }
fail() {
    echo "FAIL: $*"
    ls -la "$app/Contents/MacOS" 2>/dev/null || true
    pgrep -fl furnish || true
    exit 1
}
install_dmg() {
    rm -rf "$app"
    hdiutil attach -nobrowse -readonly -mountpoint "$RUNNER_TEMP/dmg" "$dmg" >/dev/null
    cp -R "$RUNNER_TEMP/dmg/furnish.app" /Applications/
    hdiutil detach "$RUNNER_TEMP/dmg" >/dev/null
}

install_dmg
# The Settings → About action runs the installer from the downloaded app. A runner
# can authorize with sudo instead of approving the macOS password dialog.
installer="$app/Contents/Resources/install-cli.sh"
sudo mkdir -p /usr/local/bin
# A broken link belonging to another install must survive a conflicting setup.
sudo ln -s "$RUNNER_TEMP/missing-furnish" /usr/local/bin/furnish
if /bin/sh "$installer"; then fail "overwrote an existing command"; fi
[ "$(readlink /usr/local/bin/furnish)" = "$RUNNER_TEMP/missing-furnish" ] || fail "changed an existing link"
sudo rm /usr/local/bin/furnish
sudo /bin/sh "$installer"
/bin/sh "$installer" # repeating setup should work without authorization
[ "$(command -v furnish)" = /usr/local/bin/furnish ] || fail "the Terminal command is missing"
current=$(version)
[ "$current" != "$next" ] || fail "the installed version is already $next"
echo "== $(basename "$dmg") ($current) in /Applications, the feed offers $next"
python3 -u -m http.server 8765 --bind 127.0.0.1 --directory "$feed" > "$RUNNER_TEMP/update-feed.log" 2>&1 &
server=$!
trap 'kill $server 2>/dev/null || true; pkill -f "$app/Contents/MacOS/" 2>/dev/null || true' EXIT
sleep 2
export FURNISH_UPDATE_FEED=http://127.0.0.1:8765/latest.json

gui=$(plutil -extract CFBundleExecutable raw "$app/Contents/Info.plist")
echo "== the desktop app ($gui, hidden) updates itself and restarts"
"$app/Contents/MacOS/$gui" --hidden > "$RUNNER_TEMP/gui.log" 2>&1 &
for _ in $(seq 150); do [ "$(version 2>/dev/null)" = "$next" ] && break; sleep 2; done
v=$(version)
[ "$v" = "$next" ] || { cat "$RUNNER_TEMP/gui.log"; fail "after the desktop app's update: $v, expected $next"; }
gui_version=$("$app/Contents/MacOS/$gui" --build-info | jq -r .version)
[ "$gui_version" = "$(jq -r .version "$feed/latest.json")" ] || fail "updated desktop reports $gui_version"
codesign --verify --deep --strict "$app" || fail "the updated app's signature doesn't verify"
for _ in $(seq 30); do pgrep -f "$app/Contents/MacOS/$gui" >/dev/null && break; sleep 1; done
pgrep -f "$app/Contents/MacOS/$gui" >/dev/null || fail "the desktop app didn't start again"
echo "✓ $current -> $v, and the app runs again"
pkill -f "$app/Contents/MacOS/$gui" || true
sleep 2

# The menu bar icon's menu: choose Restart to update if it offers it (the update is downloaded).
# A real mouse click opens the menu; its items are then clicked through System Events.
swiftc -O "$(dirname "$0")/ui/click.swift" -o "$RUNNER_TEMP/click"
tray_restart() {
    local pos x y w h
    pos=$(osascript -e "tell application \"System Events\" to get {position, size} of menu bar item 1 of menu bar 2 of (first process whose unix id is $1)")
    IFS=', ' read -r x y w h <<< "$pos"
    "$RUNNER_TEMP/click" $((x + w / 2)) $((y + h / 2))
    sleep 2
    osascript - "$1" "${2:-restart}" <<'OSA'
on run argv
    set thePid to (item 1 of argv) as integer
    tell application "System Events"
        set m to menu 1 of menu bar item 1 of menu bar 2 of (first process whose unix id is thePid)
        if exists menu item "Restart to update" of m then
            if item 2 of argv is "inspect" then
                key code 53
                return "ready"
            end if
            click menu item "Restart to update" of m
            return "clicked"
        end if
        set shown to "menu:"
        repeat with i in (menu items of m)
            set n to name of i
            if n is not missing value then set shown to shown & " " & n & ";"
        end repeat
        key code 53
        return shown
    end tell
end run
OSA
}

echo "== Restart to update in the menu bar icon's menu, with the window open"
install_dmg
[ "$(version)" = "$current" ] || fail "reinstalling the .dmg left $(version)"
# A one-minute interval lets this foreground process observe another release while Ready.
export FURNISH_DATA_DIR="$RUNNER_TEMP/foreground-update"
mkdir -p "$FURNISH_DATA_DIR"
echo '{"updateIntervalMinutes":1}' > "$FURNISH_DATA_DIR/settings.json"
"$app/Contents/MacOS/$gui" > "$RUNNER_TEMP/gui-restart.log" 2>&1 &
before=$!
for _ in $(seq 45); do
    r=$(tray_restart "$before" inspect 2>&1) || true
    [ "$r" = ready ] && break
    sleep 2
done
[ "$r" = ready ] || fail "the first signed download never became ready"

# Announce a later release with the old signature: its download must be attempted by the
# interval, and the superseded installer must disappear even though its replacement fails.
cp "$feed/latest.json" "$RUNNER_TEMP/first-manifest.json"
jq '.version = "99.0.0" | .platforms |= with_entries(.value.url += "?replacement")' \
    "$RUNNER_TEMP/first-manifest.json" > "$feed/new.json"
mv "$feed/new.json" "$feed/latest.json"
for _ in $(seq 100); do
    grep -q '?replacement HTTP' "$RUNNER_TEMP/update-feed.log" && break
    sleep 2
done
grep -q '?replacement HTTP' "$RUNNER_TEMP/update-feed.log" || fail "the pending updater never checked for the newer release"
r=$(tray_restart "$before" inspect)
[[ "$r" = menu:* ]] || fail "Restart still offers the superseded installer: $r"
[ "$(version)" = "$current" ] || fail "the stale update was installed"
cp "$RUNNER_TEMP/first-manifest.json" "$feed/new.json" && mv "$feed/new.json" "$feed/latest.json"
clicked=""
for _ in $(seq 60); do
    sleep 5
    # Automatic updates wait while the window is open: still the installed version.
    [ "$(version)" = "$current" ] || { cat "$RUNNER_TEMP/gui-restart.log"; fail "updated to $(version) on its own, with the window open"; }
    r=$(tray_restart "$before" 2>&1) || true
    echo "  $r"
    [ "$r" = clicked ] && { clicked=1; break; }
done
[ -n "$clicked" ] || { cat "$RUNNER_TEMP/gui-restart.log"; fail "the menu never offered Restart to update"; }
for _ in $(seq 60); do [ "$(version 2>/dev/null)" = "$next" ] && break; sleep 2; done
v=$(version)
[ "$v" = "$next" ] || { cat "$RUNNER_TEMP/gui-restart.log"; fail "after Restart to update: $v, expected $next"; }
codesign --verify --deep --strict "$app" || fail "the updated app's signature doesn't verify"
for _ in $(seq 30); do pgrep -f "$app/Contents/MacOS/$gui" | grep -vqx "$before" && break; sleep 1; done
pgrep -f "$app/Contents/MacOS/$gui" | grep -vqx "$before" || fail "the desktop app didn't start again after Restart to update"
echo "✓ $current -> $v after Restart to update, and the app runs again"
pkill -f "$app/Contents/MacOS/$gui" || true
sleep 2

echo "== furnish self-update"
install_dmg
[ "$(version)" = "$current" ] || fail "reinstalling the .dmg left $(version)"
furnish self-update
v=$(version)
[ "$v" = "$next" ] || fail "after furnish self-update: $v, expected $next"
codesign --verify --deep --strict "$app" || fail "the updated app's signature doesn't verify"
echo "✓ $current -> $v"
furnish self-update --check
