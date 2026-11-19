#!/bin/sh
# Run the UI tests (tests/ui/test.mjs) against the real furnish GUI in an Ubuntu 24.04 container.
#
#   tests/ui/run.sh [furnish-gui [replacement-furnish-gui]]   # binary built on Ubuntu 24.04 (CI); omitted: built in the container
#
# The container gets a TTY on purpose: the GUI must still elevate via pkexec, not sudo in a terminal.
# A fake pacman/sudo/pkexec on PATH records what happened, so no real packages are touched.
set -eu
# tauri-driver's port (and the next one for WebKitWebDriver): a random one unless given, so
# several runs on one machine don't share a driver or X display (the containers use the host network).
DRIVER_PORT=${DRIVER_PORT:-$(shuf -i 20000-39999 -n 1)}
export DRIVER_PORT
repo=$(cd "$(dirname "$0")/../.." && pwd)
shot="$repo/target/ui-test.png"
work=$(mktemp -d)
container=furnish-ui-test-$$
cleanup() { docker rm -f "$container" >/dev/null 2>&1 || true; rm -rf "$work"; }
trap cleanup EXIT

if [ -n "${1:-}" ]; then
    binary=$(realpath "$1")
    docker build -q --target runtime -t furnish-ui "$repo/tests/ui" >/dev/null
    image=furnish-ui
else
    docker build -q --target build -t furnish-ui-build "$repo/tests/ui" >/dev/null
    (cd "$repo/gui" && npm run -s build)
    docker run --rm --user "$(id -u):$(id -g)" -e HOME=/tmp -v "$repo:/src" -v "$HOME/.cargo/registry:/opt/cargo/registry" \
        -w /src -e CARGO_TARGET_DIR=/src/target/ui-docker furnish-ui-build \
        cargo build -q --release -p furnish-gui --features tauri/custom-protocol
    binary="$repo/target/ui-docker/release/furnish-gui"
    image=furnish-ui-build
fi

# Bind the build inputs read-only and install a copy. Like a real package, the installed
# path can be atomically replaced while a process continues running the old executable.
replacement=$(realpath "${2:-$binary}")

# Fake package manager and elevation tools, logging every call into $work.
mkdir -p "$work/bin" "$work/pkgs"
cat > "$work/bin/pacman" <<SH
#!/bin/sh
for a do shift; [ "\$a" = -- ] || set -- "\$@" "\$a"; done  # drop the end of options
case "\$1" in
  -Q) [ -f "$work/pkgs/\$2" ] && echo "\$2 \$(cat "$work/pkgs/\$2")" ;;
  -Qu) [ -f "$work/pkgs/\$2.new" ] && echo "\$2 \$(cat "$work/pkgs/\$2") -> \$(cat "$work/pkgs/\$2.new")" ;;
  # The sync database's version: what -S installs (the update if there is one, else 1.0).
  -Si) v=1.0; [ -f "$work/pkgs/\$2.new" ] && v=\$(cat "$work/pkgs/\$2.new")
       printf 'Repository      : extra\nName            : %s\nVersion         : %s\nDescription     : fake\n' "\$2" "\$v" ;;
  -S) sleep 2  # slow enough to observe live progress
      for p do :; done  # the package: the last argument
      if [ -f "$work/fail-once-\$p" ]; then
        rm "$work/fail-once-\$p"; echo "error: failed to install \$p" >&2; exit 1
      fi
      [ "\$p" = broken ] && { echo "error: failed to install broken" >&2; exit 1; }
      if [ -f "$work/pkgs/\$p.new" ]; then mv "$work/pkgs/\$p.new" "$work/pkgs/\$p"; else echo 1.0 > "$work/pkgs/\$p"; fi
      if [ "\$p" = brew ]; then cp "$work/fake-brew" "$work/bin/brew"; fi ;;
  -R) rm "$work/pkgs/\$3"; if [ "\$3" = brew ]; then rm -f "$work/bin/brew"; fi ;;
esac
SH
# Installing the "brew" package provides this fake Homebrew (see the homebrew fixture app).
cat > "$work/fake-brew" <<SH
#!/bin/sh
mkdir -p "$work/brew-pkgs"
case "\$1" in
  list) [ -f "$work/brew-pkgs/\$3" ] && echo "\$3 1.0" ;;
  install) touch "$work/brew-pkgs/\$2" ;;
  uninstall) rm -f "$work/brew-pkgs/\$2" ;;
esac
SH
chmod +x "$work/fake-brew"
for tool in sudo pkexec; do
    printf '#!/bin/sh\necho "$*" >> %s/%s.log\nexec "$@"\n' "$work" "$tool" > "$work/bin/$tool"
done
# Like a refused polkit dialog while the test has created $work/deny.
sed -i '2i [ -e '"$work"'/deny ] && { echo "$*" >> '"$work"'/denied.log; echo "Error executing command as another user: Not authorized" >&2; exit 127; }' "$work/bin/pkexec"
# Like a real sudo without cached credentials: `sudo -n` fails, so only the terminal prompt is left.
sed -i '2i [ "$1" = -n ] && { echo "$*" >> '"$work"'/sudo.log; exit 1; }' "$work/bin/sudo"
# The elsewhere app's program, as npm or a vendor's installer would leave it: installed outside furnish.
printf '#!/bin/sh\n' > "$work/bin/elsewhere-tool"
# The browser: records what furnish opens.
printf '#!/bin/sh\necho "$1" >> %s/browser.log\n' "$work" > "$work/bin/browser"
# Record the separate file-manager action; directories must not go through FURNISH_BROWSER.
printf '#!/bin/sh\n[ "$#" = 1 ] && [ -d "$1" ] || exit 1\nprintf "%%s\\n" "$1" >> %s/folders.log\n' "$work" > "$work/bin/xdg-open"
chmod +x "$work/bin/"*

# The fixture collection (already added) and a second repository for the add/remove test.
repo_from_fixture() {
    cp -r "$repo/tests/fixtures/collection" "$1"
    git -C "$1" init -q && git -C "$1" add -A && git -C "$1" -c user.name=t -c user.email=t@t commit -qm init
}
mkdir -p "$work/data/collections"
repo_from_fixture "$work/origin-src"
# An app whose latest version fails to install; the one before works (falling back).
flaky() {
    printf 'metadata: { furnish: { description: Its latest version is broken, categories: [Test] } }\nresources:\n  - name: flaky\n    type: Furnish/Package\n    properties: { name: %s }\n' "$1" > "$work/origin-src/apps/flaky.dsc.yaml"
    git -C "$work/origin-src" add -A && git -C "$work/origin-src" -c user.name=t -c user.email=t@t commit -qm "flaky: $1"
}
flaky flaky; flaky broken
# Required versions: "secure" is installed at 1.0, which its collection accepts for now (the test
# raises the requirement later); "outdated" needs a version pacman doesn't offer (it offers 1.0).
required() {
    printf 'metadata: { furnish: { description: %s, categories: [Test] } }\nresources:\n  - name: %s\n    type: Furnish/Package\n    properties: { name: %s, versions: [%s] }\n' "$3" "$1" "$1" "$2" > "$work/origin-src/apps/$1.dsc.yaml"
}
# An absent NixOS entry can be previewed on any Linux runner without a Nix installation.
cat > "$work/origin-src/apps/nixos-preview.dsc.yaml" <<'YAML'
resources:
  - name: NixOS entry
    type: Furnish/NixOS
    properties: { name: gui-preview, _exist: false }
YAML
cat > "$work/origin-src/apps/session-notice.dsc.yaml" <<YAML
metadata:
  furnish:
    description: Exercise an installer that changes the login environment
    requiresNewSession: true
resources:
  - name: Session setup
    type: Furnish/File
    properties: { path: "$work/session-notice", content: ready }
YAML
cat > "$work/origin-src/apps/preflight-settings.dsc.yaml" <<YAML
resources:
  - name: Earlier step
    type: Furnish/File
    properties: { path: "$work/preflight-earlier", content: ready }
  - name: Write settings
    type: Furnish/Value
    properties: { path: "$work/preflight.json", location: '$.enabled', value: true, requiresClosed: [furnish-ui-cache, absent-cache-writer] }
YAML
cat > "$work/origin-src/apps/preflight-dependent.dsc.yaml" <<YAML
metadata: { furnish: { dependencies: [{ app: preflight-settings, required: true }] } }
resources:
  - name: Dependent
    type: Furnish/File
    properties: { path: "$work/preflight-dependent", content: ready }
YAML
cat > "$work/origin-src/apps/preflight-free.dsc.yaml" <<YAML
resources:
  - name: Independent
    type: Furnish/File
    properties: { path: "$work/preflight-free", content: ready }
YAML
required secure "'>=1.0'" 'Its collection requires recent versions'
required outdated "'>=2.0'" 'Needs a version nobody offers'
git -C "$work/origin-src" add -A && git -C "$work/origin-src" -c user.name=t -c user.email=t@t commit -qm "required versions"
echo 1.0 > "$work/pkgs/secure"
git clone -q --bare "$work/origin-src" "$work/origin.git"
git clone -q "$work/origin.git" "$work/data/collections/fixture"
# The origin moves on after the app's clone: updating on launch must bring in this app.
cat > "$work/origin-src/apps/fresh.dsc.yaml" <<'YAML'
metadata:
  furnish:
    description: Arrived with a collection update
    categories: [Test]
resources:
  - name: fresh
    type: Furnish/Package
    properties:
      name: fresh
YAML
cat > "$work/origin-src/kits/recovery.yaml" <<'YAML'
description: Two apps for testing a partially successful kit install
items:
  - { app: fresh, required: true }
  - { app: toolbox, required: true }
YAML
git -C "$work/origin-src" add -A && git -C "$work/origin-src" -c user.name=t -c user.email=t@t commit -qm fresh
git -C "$work/origin-src" push -q "$work/origin.git" HEAD
repo_from_fixture "$work/second"
GIT_COMMITTER_DATE=2022-01-01T00:00:00Z git -C "$work/second" -c user.name=t -c user.email=t@t tag -a release-2 -m release-2
git -C "$work/second" -c user.name=t -c user.email=t@t commit -q --allow-empty -m stable
git -C "$work/second" branch stable
GIT_COMMITTER_DATE=2023-01-01T00:00:00Z git -C "$work/second" -c user.name=t -c user.email=t@t tag -a release-1 -m release-1
git -C "$work/second" -c user.name=t -c user.email=t@t commit -q --allow-empty -m development

# Install a launcher in the package search path. Keep its command and actions distinguishable
# from the defaults so the desktop override must use the installed package's entry.
mkdir -p "$work/system-share/applications" "$work/user-share"
cat > "$work/system-share/applications/furnish.desktop" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=furnish
Name[de]=Paketverwaltung
Icon=furnish-gui
Exec=/usr/local/bin/furnish-gui %u
MimeType=x-scheme-handler/furnish;
StartupWMClass=furnish
Actions=Hidden;

[Desktop Action Hidden]
Name=Start in tray
Exec=/usr/local/bin/furnish-gui --hidden
DESKTOP

# An admin policy: the fixture collection is managed, the update interval locked, the app branded.
cat > "$work/policy.json" <<JSON
{"settings": {"updateIntervalMinutes": {"value": 30, "locked": true}, "reporting": {"value": "errors", "locked": true}},
 "collections": {"preset": [{"name": "fixture", "url": "$work/origin.git", "tracking": {"mode": "default"}}]},
 "branding": {"name": "Acme Apps", "logo": "fixture/branding/logo.svg", "accent": "#e30613"},
 "parameters": {"fixture": {"role": {"value": "developer"}}}}
JSON

# A feed with a newer furnish (src/selfupdate.rs), served on the host: the app (built from source,
# so not installed in a way it updates itself) only shows that the version is out.
mkdir -p "$work/feed"
echo '{"version": "99.0.0", "notes": "https://example.org/notes", "platforms": {}}' > "$work/feed/latest.json"
FEED_PORT=$((DRIVER_PORT + 2))
python3 -m http.server "$FEED_PORT" --bind 127.0.0.1 --directory "$work/feed" >"$work/feed.log" 2>&1 &
feed_server=$!
trap 'kill $feed_server 2>/dev/null || true; cleanup' EXIT

# DSC's resources, if the build has them beside it (Tauri copies them there), where an installed app has them.
dsc_dir="$(dirname "$binary")/dsc"
dsc_mount=$dsc_dir:/usr/local/lib/furnish/dsc:ro
[ -d "$dsc_dir" ] || dsc_mount=$work:/nonexistent:ro
# $work is mounted at the same path, so paths in logs and inputs mean the same inside and out.
docker run -d -t --name "$container" --network host --user "$(id -u):$(id -g)" -e HOME=/tmp \
    -e XDG_DATA_HOME="$work/user-share" -e XDG_DATA_DIRS="$work/system-share:/usr/local/share:/usr/share" -e FURNISH_DATA_DIR="$work/data" -e FURNISH_POLICY_FILE="$work/policy.json" -e FURNISH_LOAD_SAMPLE_SECONDS=1 -e FURNISH_BROWSER="$work/bin/browser" -e FURNISH_UPDATE_FEED="http://127.0.0.1:$FEED_PORT/latest.json" -e PATH="$work/bin:/usr/local/bin:/usr/bin:/bin" \
    -v "$work:$work" -v "$binary:/fixtures/running:ro" -v "$replacement:/fixtures/replacement:ro" -v "$dsc_mount" "$image" \
    sh -c 'Xvfb :'"$((DRIVER_PORT % 10000 + 100))"' -screen 0 1280x800x24 >/dev/null 2>&1 & export DISPLAY=:'"$((DRIVER_PORT % 10000 + 100))"'; exec tauri-driver --port '"$DRIVER_PORT"' --native-port '"$((DRIVER_PORT + 1))" >/dev/null

docker exec --user 0 "$container" install /fixtures/running /usr/local/bin/furnish-gui

# A second launch path lets the suite exercise a Nix-managed installation of the same binary.
docker exec --user 0 "$container" install -D /usr/local/bin/furnish-gui /nix/store/fixture-furnish/bin/furnish-gui

# Sampling every second makes the load window 30 s instead of 5 minutes.
if node "$repo/tests/ui/test.mjs" "$work" "$shot" "$container" "${2:+replace}"; then
    echo "UI tests passed"
else
    echo "--- container log"; docker logs "$container" 2>&1 | grep -v -e MESA -e '^$' | tail -30
    exit 1
fi
