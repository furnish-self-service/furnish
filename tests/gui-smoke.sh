#!/bin/sh
# GUI smoke test: run the real furnish GUI in an Arch container under Xvfb with the test
# fixture collection, then screenshot it. Proves the app starts, renders, and gets app status
# from the real engine. Touches nothing on the host desktop.
#
#   tests/gui-smoke.sh [path/to/furnish-gui] [screenshot.png]
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
binary=$(realpath "${1:-$repo/target/release/furnish-gui}")
shot=$(realpath -m "${2:-$repo/target/gui-smoke.png}")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Data dir with the fixture collection already added (collections are plain git clones).
mkdir -p "$work/data/collections"
cp -r "$repo/tests/fixtures/collection" "$work/data/collections/fixture"
git -C "$work/data/collections/fixture" init -q
git -C "$work/data/collections/fixture" add -A
git -C "$work/data/collections/fixture" -c user.name=t -c user.email=t@t commit -qm init
git -C "$work/data/collections/fixture" remote add origin https://example.invalid/fixture.git

docker build -q -t furnish-gui-smoke - >/dev/null <<'EOF'
FROM archlinux:latest
RUN pacman -Syu --noconfirm --needed webkit2gtk-4.1 xorg-server-xvfb imagemagick git ttf-dejavu \
    && pacman -Scc --noconfirm
EOF

# Run as the invoking user: the data dir must be owned by the user running furnish.
docker run --rm --user "$(id -u):$(id -g)" -e HOME=/tmp -v "$binary:/usr/local/bin/furnish-gui:ro" -v "$work/data:/data" -v "$(dirname "$shot"):/out" \
    -e FURNISH_DATA_DIR=/data -e WEBKIT_DISABLE_DMABUF_RENDERER=1 furnish-gui-smoke sh -c "
    Xvfb :99 -screen 0 1280x800x24 >/dev/null 2>&1 &
    sleep 1
    DISPLAY=:99 furnish-gui >/tmp/gui.log 2>&1 &
    sleep 10
    DISPLAY=:99 import -window root /out/$(basename "$shot")
    cat /tmp/gui.log"
echo "screenshot: $shot"
