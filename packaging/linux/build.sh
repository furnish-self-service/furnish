#!/bin/sh
# Build the Linux packages (.deb, .rpm) on Ubuntu 22.04, so the app runs on glibc 2.35 and newer;
# the bundled furnish CLI is static and runs anywhere. Output: target/jammy/release/bundle/.
#
#   docker run --rm -v "$PWD:/src" -w /src -e HOST_UID=$(id -u) [-e FURNISH_VERSION=…] ubuntu:22.04 packaging/linux/build.sh
set -eu
# Hand the build output back to the host user, also when the build fails.
[ -z "${HOST_UID:-}" ] || trap 'chown -R "$HOST_UID" target/jammy gui/dist gui/node_modules gui/src-tauri/binaries gui/src-tauri/dsc .cache/dsc 2>/dev/null || true' EXIT
export DEBIAN_FRONTEND=noninteractive
# Retry transient archive errors inside the Jammy build container.
echo 'Acquire::Retries "3"; Acquire::http::Timeout "20";' > /etc/apt/apt.conf.d/80-furnish-retries
apt-get update -qq
apt-get install -y -qq curl ca-certificates build-essential pkg-config git musl-tools \
    libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev patchelf >/dev/null
curl -fsSL https://deb.nodesource.com/setup_22.x | bash - >/dev/null
apt-get install -y -qq nodejs >/dev/null
curl -sSf https://sh.rustup.rs | sh -s -- -y -q --profile minimal --target x86_64-unknown-linux-musl
. "$HOME/.cargo/env"

export CARGO_TARGET_DIR="$PWD/target/jammy"  # apart from builds for the host
cd gui
npm ci --silent
# FURNISH_VERSION (e.g. 0.1.0-alpha.42) overrides the version, for alpha builds.
FURNISH_CLI_TARGET=x86_64-unknown-linux-musl npm run bundle -- --bundles deb,rpm \
    ${FURNISH_VERSION:+--config "{\"version\": \"$FURNISH_VERSION\"}"}
cd ..
if [ -n "${FURNISH_BUILD_REVISION:-}" ] && [ -n "${FURNISH_VERSION:-}" ] && [ -n "${SOURCE_DATE_EPOCH:-}" ]; then
    # Query the package's actual desktop executable on the oldest supported glibc.
    extracted=$(mktemp -d)
    dpkg-deb -x target/jammy/release/bundle/deb/*.deb "$extracted"
    node scripts/verify-desktop-build.mjs "$extracted/usr/bin/furnish-gui" "$FURNISH_VERSION"
    rm -rf "$extracted"
fi
ls target/jammy/release/bundle/deb/*.deb target/jammy/release/bundle/rpm/*.rpm
