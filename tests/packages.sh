#!/bin/sh
# Install the Linux packages on a distro and check both programs can run there: the static
# furnish CLI runs, and furnish-gui finds every library (and glibc version) it needs.
#
#   tests/packages.sh <image> <dir with the .deb and .rpm>     e.g. debian:12 target/jammy/release/bundle
set -eu
image=${1:?usage: $0 <image> <bundle dir>}
dir=$(realpath "${2:?usage: $0 <image> <bundle dir>}")
case "$image" in
    debian*|ubuntu*) install='apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq /p/deb/*.deb >/dev/null' ;;
    fedora*) install='dnf install -y -q /p/rpm/*.rpm >/dev/null' ;;
    # Enterprise Linux has WebKitGTK 4.1 from version 10, with EPEL and CRB (EL9 only has 4.0).
    rockylinux*|almalinux*) install='dnf install -y -q epel-release >/dev/null && dnf config-manager --set-enabled crb && dnf install -y -q /p/rpm/*.rpm >/dev/null' ;;
    opensuse*) install='zypper -n -q --no-gpg-checks install --allow-unsigned-rpm /p/rpm/*.rpm >/dev/null' ;;
    *) echo "unknown image: $image" >&2; exit 1 ;;
esac
docker run --rm -v "$dir:/p:ro" "$image" sh -c "
    set -e
    $install
    furnish --version
    missing=\$(ldd /usr/bin/furnish-gui | grep 'not found' || true)
    [ -z \"\$missing\" ] || { echo \"furnish-gui can't start: \$missing\" >&2; exit 1; }
    echo 'packages OK on $image'
"
