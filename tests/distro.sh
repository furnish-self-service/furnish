#!/bin/sh
# Run the real end-to-end test inside a Linux distro container. Same command in CI and locally.
#
#   tests/distro.sh <image> <path/to/static/furnish>
#   e.g. tests/distro.sh archlinux:latest target/musl-docker/release/furnish
set -eu
image=${1:?usage: $0 <image> <furnish-binary>}
binary=$(realpath "${2:?usage: $0 <image> <furnish-binary>}")
repo=$(cd "$(dirname "$0")/.." && pwd)

# Minimal prerequisites per distro: git (collections are git repos) and bash (test script).
case "$image" in
    ubuntu*|debian*) setup='apt-get update -qq && apt-get install -y -qq git >/dev/null' ;;
    fedora*)         setup='dnf install -y -q git' ;;
    opensuse*)       setup='zypper -n -q install git' ;;
    archlinux*)      setup='pacman -Sy --noconfirm --needed -q git >/dev/null' ;;
    nixos/nix*)      setup='command -v git || nix-env -iA nixpkgs.git' ;;
    *) echo "unknown image: $image" >&2; exit 1 ;;
esac

docker run --rm -v "$repo:/src:ro" -v "$binary:/usr/local/bin/furnish:ro" -w /src "$image" \
    sh -c "$setup && bash tests/e2e.sh --real /usr/local/bin/furnish"
