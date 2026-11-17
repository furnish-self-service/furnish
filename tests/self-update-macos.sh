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
version() { "$app/Contents/MacOS/furnish" --version | sed 's/^furnish //'; }
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
current=$(version)
[ "$current" != "$next" ] || fail "the installed version is already $next"
echo "== $(basename "$dmg") ($current) in /Applications, the feed offers $next"
python3 -m http.server 8765 --bind 127.0.0.1 --directory "$feed" >/dev/null 2>&1 &
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
    osascript - "$1" <<'OSA'
on run argv
    set thePid to (item 1 of argv) as integer
    tell application "System Events"
        set m to menu 1 of menu bar item 1 of menu bar 2 of (first process whose unix id is thePid)
        if exists menu item "Restart to update" of m then
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
"$app/Contents/MacOS/$gui" > "$RUNNER_TEMP/gui-restart.log" 2>&1 &
before=$!
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
"$app/Contents/MacOS/furnish" self-update
v=$(version)
[ "$v" = "$next" ] || fail "after furnish self-update: $v, expected $next"
codesign --verify --deep --strict "$app" || fail "the updated app's signature doesn't verify"
echo "✓ $current -> $v"
"$app/Contents/MacOS/furnish" self-update --check
