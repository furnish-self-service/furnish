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
  -S) sleep 2; echo 1.0 > "$work/pkgs/\$4" ;;  # slow enough to observe live progress
  -R) rm "$work/pkgs/\$3" ;;
esac
SH
for tool in sudo pkexec; do
    printf '#!/bin/sh\necho "$*" >> %s/%s.log\nexec "$@"\n' "$work" "$tool" > "$work/bin/$tool"
done
# Like a real sudo without cached credentials: `sudo -n` fails, so only the terminal prompt is left.
sed -i '2i [ "$1" = -n ] && { echo "$*" >> '"$work"'/sudo.log; exit 1; }' "$work/bin/sudo"
chmod +x "$work/bin/"*

# The fixture collection (already added) and a second repository for the add/remove test.
repo_from_fixture() {
    cp -r "$repo/tests/fixtures/collection" "$1"
    git -C "$1" init -q && git -C "$1" add -A && git -C "$1" -c user.name=t -c user.email=t@t commit -qm init
}
mkdir -p "$work/data/collections"
repo_from_fixture "$work/data/collections/fixture"
git -C "$work/data/collections/fixture" remote add origin https://example.invalid/fixture.git
repo_from_fixture "$work/second"

# $work is mounted at the same path, so paths in logs and inputs mean the same inside and out.
docker run -d -t --name "$container" --network host --user "$(id -u):$(id -g)" -e HOME=/tmp \
    -e FURNISH_DATA_DIR="$work/data" -e PATH="$work/bin:/usr/local/bin:/usr/bin:/bin" \
    -v "$work:$work" -v "$binary:/usr/local/bin/furnish-gui:ro" "$image" \
    sh -c 'Xvfb :99 -screen 0 1280x800x24 >/dev/null 2>&1 & export DISPLAY=:99; exec tauri-driver --port 4444' >/dev/null

if node "$repo/tests/ui/test.mjs" "$work" "$shot"; then
    echo "UI tests passed"
else
    echo "--- container log"; docker logs "$container" 2>&1 | grep -v -e MESA -e '^$' | tail -30
    exit 1
fi
