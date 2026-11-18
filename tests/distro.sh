#!/bin/sh
# Run the real end-to-end test inside a Linux distro container. Same command in CI and locally.
#
#   tests/distro.sh <image> <path/to/static/furnish>
#   e.g. tests/distro.sh archlinux:latest target/musl-docker/release/furnish
set -eu
image=${1:?usage: $0 <image> <furnish-binary>}
binary=$(realpath "${2:?usage: $0 <image> <furnish-binary>}")
repo=$(cd "$(dirname "$0")/.." && pwd)

# Prerequisites per distro: git (collections are git repos), git-lfs (LFS test), bash (test
# script) and, for rpm distros, rpm-build to build the installer test file (.deb uses dpkg-deb).
case "$image" in
    ubuntu*|debian*) setup='apt-get update -qq && apt-get install -y -qq git git-lfs >/dev/null' ;;
    fedora*)         setup='dnf install -y -q git git-lfs rpm-build' ;;
    opensuse*)       setup='zypper -n -q install git git-lfs rpm-build' ;;
    archlinux*)      setup='pacman -Sy --noconfirm --needed -q git git-lfs >/dev/null' ;;
    nixos/nix*)      setup='command -v git || nix-env -iA nixpkgs.git' ;;
    *) echo "unknown image: $image" >&2; exit 1 ;;
esac

sh "$repo/tests/distro-container.sh" "$image" "$setup && bash tests/e2e.sh --real /usr/local/bin/furnish" \
    -v "$repo:/src:ro" -v "$binary:/usr/local/bin/furnish:ro" -w /src
