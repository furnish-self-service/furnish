#!/bin/bash
# furnish updates itself on macOS: the app from the .dmg (this build), in /Applications, takes the
# next version from a feed on this computer, first in the desktop app (automatic, hidden, it
# restarts into the new version), then with `furnish self-update`. Run on a throwaway Mac (CI).
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

echo "== furnish self-update"
install_dmg
[ "$(version)" = "$current" ] || fail "reinstalling the .dmg left $(version)"
"$app/Contents/MacOS/furnish" self-update
v=$(version)
[ "$v" = "$next" ] || fail "after furnish self-update: $v, expected $next"
codesign --verify --deep --strict "$app" || fail "the updated app's signature doesn't verify"
echo "✓ $current -> $v"
"$app/Contents/MacOS/furnish" self-update --check
