#!/bin/sh
# Install the Linux packages on a distro and check both programs can run there: the static
# furnish CLI runs, git came along, furnish-gui finds every library (and glibc version) it needs,
# and an app installs with DSC's resources from the package (/usr/lib/furnish/dsc).
#
#   tests/packages.sh <image> <dir with the .deb and .rpm>     e.g. debian:12 target/jammy/release/bundle
set -eu
image=${1:?usage: $0 <image> <bundle dir>}
dir=$(realpath "${2:?usage: $0 <image> <bundle dir>}")
case "$image" in
    debian*|ubuntu*) install='apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install --no-install-recommends -y -qq /p/deb/*.deb >/dev/null' ;;
    fedora*) install='dnf install -y -q /p/rpm/*.rpm >/dev/null' ;;
    # Enterprise Linux has WebKitGTK 4.1 from version 10, with EPEL and CRB (EL9 only has 4.0).
    rockylinux*|almalinux*) install='dnf install -y -q epel-release >/dev/null && dnf config-manager --set-enabled crb && dnf install -y -q /p/rpm/*.rpm >/dev/null' ;;
    # The Leap mirror redirector can stall dependency downloads on hosted runners.
    # Use the same signed repositories on openSUSE's CDN and keep zypper progress in the job log.
    opensuse/leap:15.6) install='
        sed -i "s|^baseurl=http://download.opensuse.org/|baseurl=https://downloadcontentcdn.opensuse.org/|" /etc/zypp/repos.d/*.repo
        zypper -n --no-gpg-checks install --allow-unsigned-rpm /p/rpm/*.rpm' ;;
    opensuse*) install='zypper -n -q --no-gpg-checks install --allow-unsigned-rpm /p/rpm/*.rpm >/dev/null' ;;
    *) echo "unknown image: $image" >&2; exit 1 ;;
esac
sh "$(dirname "$0")/distro-container.sh" "$image" "
    $install
    furnish --version
    furnish-gui --build-info | grep -F '\"target\":\"x86_64-unknown-linux-gnu\"'
    git lfs version >/dev/null || { echo 'the package did not bring git-lfs' >&2; exit 1; }
    git --version >/dev/null || { echo 'the package did not bring git' >&2; exit 1; }
    missing=\$(ldd /usr/bin/furnish-gui | grep 'not found' || true)
    [ -z \"\$missing\" ] || { echo \"furnish-gui can't start: \$missing\" >&2; exit 1; }
    mkdir -p /tmp/c/apps && cd /tmp/c
    printf 'resources:\\n  - name: file\\n    type: Microsoft.Filesystem.File/Content\\n    properties: {path: /etc/furnish-test, content: furnish}\\n' > apps/file.dsc.yaml
    git init -q && git add -A && git -c user.name=t -c user.email=t@t commit -qm apps
    furnish collection add /tmp/c --name c >/dev/null && furnish install file
    [ \"\$(cat /etc/furnish-test)\" = furnish ] || { echo 'File/Content from /usr/lib/furnish/dsc did not run' >&2; exit 1; }
    echo 'packages OK on $image'
" -v "$dir:/p:ro"
