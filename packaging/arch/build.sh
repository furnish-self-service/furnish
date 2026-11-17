#!/bin/sh
# Build the Arch Linux package from the .deb (packaging/linux/build.sh), in an archlinux container.
# Output: bundle/arch/furnish-<pkgver>-1-x86_64.pkg.tar.zst next to bundle/deb/ (unsigned; publish.sh signs it).
#
#   docker run --rm -v "$PWD:/src" -w /src -e HOST_UID=$(id -u) archlinux:latest \
#       packaging/arch/build.sh target/jammy/release/bundle/deb/furnish_0.1.0_amd64.deb
set -eu
deb=${1:?usage: $0 <furnish_<version>_amd64.deb>}
out=$(cd "$(dirname "$deb")/.." && pwd)/arch
here=$(cd "$(dirname "$0")" && pwd)
debver=$(basename "$deb" | sed -E 's/^furnish_(.*)_amd64\.deb$/\1/')
# pacman's versions can't contain "-" (nor should they "+"): 0.1.0+alpha.42 becomes 0.1.0alpha.42.
pkgver=$(echo "$debver" | sed -E 's/[+-]alpha/alpha/')
pacman -Syu --noconfirm --needed --quiet base-devel >/dev/null

work=$(mktemp -d)
sed -e "s/^pkgver=.*/pkgver=$pkgver/" -e "s/^sha256sums=.*/sha256sums=('$(sha256sum "$deb" | cut -d' ' -f1)')/" \
    "$here/PKGBUILD" > "$work/PKGBUILD"
cp "$deb" "$work/furnish_${debver}_amd64.deb"
# makepkg doesn't run as root.
id builder >/dev/null 2>&1 || useradd -m builder
chown -R builder "$work"
su builder -c "cd '$work' && PACKAGER=\"furnish <noreply@furnish-self-service.github.io>\" makepkg --nodeps --noconfirm" >/dev/null
mkdir -p "$out"
cp "$work"/furnish-*.pkg.tar.zst "$out/"
[ -z "${HOST_UID:-}" ] || chown -R "$HOST_UID" "$out"
ls "$out"/furnish-"$pkgver"-*.pkg.tar.zst
