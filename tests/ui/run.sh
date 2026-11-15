#!/bin/sh
# Run the UI tests (tests/ui/test.mjs) against the real furnish GUI in an Ubuntu 24.04 container.
#
#   tests/ui/run.sh [furnish-gui]   # binary built on Ubuntu 24.04 (CI); omitted: built in the container
#
# The container gets a TTY on purpose: the GUI must still elevate via pkexec, not sudo in a terminal.
# A fake pacman/sudo/pkexec on PATH records what happened, so no real packages are touched.
set -eu
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

# Fake package manager and elevation tools, logging every call into $work.
mkdir -p "$work/bin" "$work/pkgs"
cat > "$work/bin/pacman" <<SH
#!/bin/sh
case "\$1" in
  -Q) [ -f "$work/pkgs/\$2" ] && echo "\$2 \$(cat "$work/pkgs/\$2")" ;;
  -Qu) [ -f "$work/pkgs/\$2.new" ] && echo "\$2 \$(cat "$work/pkgs/\$2") -> \$(cat "$work/pkgs/\$2.new")" ;;
  -S) sleep 2  # slow enough to observe live progress
      [ "\$4" = broken ] && { echo "error: failed to install broken" >&2; exit 1; }
      if [ -f "$work/pkgs/\$4.new" ]; then mv "$work/pkgs/\$4.new" "$work/pkgs/\$4"; else echo 1.0 > "$work/pkgs/\$4"; fi
      if [ "\$4" = brew ]; then cp "$work/fake-brew" "$work/bin/brew"; fi ;;
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
sed -i '2i [ -e '"$work"'/deny ] && { echo "Error executing command as another user: Not authorized" >&2; exit 127; }' "$work/bin/pkexec"
# Like a real sudo without cached credentials: `sudo -n` fails, so only the terminal prompt is left.
sed -i '2i [ "$1" = -n ] && { echo "$*" >> '"$work"'/sudo.log; exit 1; }' "$work/bin/sudo"
# The browser: records what furnish opens.
printf '#!/bin/sh\necho "$1" >> %s/browser.log\n' "$work" > "$work/bin/browser"
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
git -C "$work/origin-src" add -A && git -C "$work/origin-src" -c user.name=t -c user.email=t@t commit -qm fresh
git -C "$work/origin-src" push -q "$work/origin.git" HEAD
repo_from_fixture "$work/second"

# An admin policy: the fixture collection is managed, the update interval locked, the app branded.
cat > "$work/policy.json" <<JSON
{"settings": {"updateIntervalMinutes": {"value": 30, "locked": true}},
 "collections": {"preset": [{"name": "fixture", "url": "$work/origin.git"}]},
 "branding": {"name": "Acme Apps", "logo": "fixture/branding/logo.svg", "accent": "#e30613"},
 "parameters": {"fixture": {"role": {"value": "developer"}}}}
JSON

# $work is mounted at the same path, so paths in logs and inputs mean the same inside and out.
docker run -d -t --name "$container" --network host --user "$(id -u):$(id -g)" -e HOME=/tmp \
    -e FURNISH_DATA_DIR="$work/data" -e FURNISH_POLICY_FILE="$work/policy.json" -e FURNISH_LOAD_SAMPLE_SECONDS=1 -e FURNISH_BROWSER="$work/bin/browser" -e PATH="$work/bin:/usr/local/bin:/usr/bin:/bin" \
    -v "$work:$work" -v "$binary:/usr/local/bin/furnish-gui:ro" "$image" \
    sh -c 'Xvfb :99 -screen 0 1280x800x24 >/dev/null 2>&1 & export DISPLAY=:99; exec tauri-driver --port 4444' >/dev/null

# Sampling every second makes the load window 30 s instead of 5 minutes.
if node "$repo/tests/ui/test.mjs" "$work" "$shot" "$container"; then
    echo "UI tests passed"
else
    echo "--- container log"; docker logs "$container" 2>&1 | grep -v -e MESA -e '^$' | tail -30
    exit 1
fi
