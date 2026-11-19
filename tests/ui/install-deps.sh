#!/bin/sh
# Install packages in the Ubuntu WebDriver image. Fetch before configuring packages
# so only an index or download failure can trigger the archive mirror fallback.
set -eu
export DEBIAN_FRONTEND=noninteractive

fetch() {
    apt-get update --error-on=any -qq &&
        apt-get install --download-only -y -qq --no-install-recommends "$@"
}

sources=/etc/apt/sources.list.d/ubuntu.sources
ubuntu_uri='^URIs: (https?)://(archive|security)[.]ubuntu[.]com/ubuntu/?$'
fetch "$@" || {
    status=$?
    [ -f "$sources" ] && grep -Eq "$ubuntu_uri" "$sources" || exit "$status"
    echo "Retrying Ubuntu package downloads from the kernel.org archive mirror" >&2
    original=$(mktemp)
    cp -p "$sources" "$original"
    trap 'mv "$original" "$sources"' EXIT
    sed -Ei "s#$ubuntu_uri#URIs: \1://mirrors.edge.kernel.org/ubuntu/#" "$sources"
    fetch "$@"
}
apt-get install --no-download -y -qq --no-install-recommends "$@"
rm -rf /var/lib/apt/lists/*
