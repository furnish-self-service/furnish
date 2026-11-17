#!/bin/sh
# Install the Linux packages (.deb, .rpm, .pkg.tar.zst) on a distro and check both programs can run there: the static
# furnish CLI runs, git came along, furnish-gui finds every library (and glibc version) it needs,
# and an app installs with DSC's resources from the package (/usr/lib/furnish/dsc).
#
#   tests/packages.sh <image> <dir with the .deb, .rpm and .pkg.tar.zst>     e.g. debian:12 target/jammy/release/bundle
set -eu
image=${1:?usage: $0 <image> <bundle dir>}
dir=$(realpath "${2:?usage: $0 <image> <bundle dir>}")
case "$image" in
    debian*|ubuntu*) install='apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq /p/deb/*.deb >/dev/null' ;;
    fedora*) install='dnf install -y -q /p/rpm/*.rpm >/dev/null' ;;
    # Enterprise Linux has WebKitGTK 4.1 from version 10, with EPEL and CRB (EL9 only has 4.0).
    rockylinux*|almalinux*) install='dnf install -y -q epel-release >/dev/null && dnf config-manager --set-enabled crb && dnf install -y -q /p/rpm/*.rpm >/dev/null' ;;
    archlinux*) install='pacman -Syu --noconfirm >/dev/null && pacman -U --noconfirm /p/arch/*.pkg.tar.zst >/dev/null' ;;
    opensuse*) install='zypper -n -q --no-gpg-checks install --allow-unsigned-rpm /p/rpm/*.rpm >/dev/null' ;;
    *) echo "unknown image: $image" >&2; exit 1 ;;
esac
docker run --rm -v "$dir:/p:ro" "$image" sh -c "
    set -e
    $install
    furnish --version
    git --version >/dev/null || { echo 'the package did not bring git' >&2; exit 1; }
    missing=\$(ldd /usr/bin/furnish-gui | grep 'not found' || true)
    [ -z \"\$missing\" ] || { echo \"furnish-gui can't start: \$missing\" >&2; exit 1; }
    mkdir -p /tmp/c/apps && cd /tmp/c
    printf 'resources:\\n  - name: file\\n    type: Microsoft.Filesystem.File/Content\\n    properties: {path: /etc/furnish-test, content: furnish}\\n' > apps/file.dsc.yaml
    git init -q && git add -A && git -c user.name=t -c user.email=t@t commit -qm apps
    furnish collection add /tmp/c --name c >/dev/null && furnish install file
    [ \"\$(cat /etc/furnish-test)\" = furnish ] || { echo 'File/Content from /usr/lib/furnish/dsc did not run' >&2; exit 1; }
    echo 'packages OK on $image'
"
