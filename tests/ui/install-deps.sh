#!/bin/sh
# Install the desktop build and WebDriver dependencies from Ubuntu's signed archives.
set -eu
export DEBIAN_FRONTEND=noninteractive

sources=/etc/apt/sources.list.d/ubuntu.sources
apt() {
    # Scope resolution to the Ubuntu packages we need. Keep unrelated runner indexes.
    timeout "$deadline" apt-get -o Dir::Etc::sourcelist="$sources" -o Dir::Etc::sourceparts=- \
        -o APT::Get::List-Cleanup=false -o Acquire::Retries=0 \
        -o Acquire::http::Timeout=15 -o Acquire::https::Timeout=15 "$@"
}
fetch() {
    apt update --error-on=any -qq && apt install --download-only -y -qq "$@"
}

# Bound the primary attempt even if apt encounters a timeout for every package.
deadline=3m
fetch "$@" || {
    status=$?
    fallback=$(mktemp --suffix=.sources)
    trap 'rm -f "$fallback"' EXIT
    sed -E \
        -e 's#^URIs: (https?)://(archive|security)[.]ubuntu[.]com/ubuntu/?$#URIs: \1://mirrors.edge.kernel.org/ubuntu/#' \
        -e 's#^URIs: mirror\+file:/etc/apt/apt-mirrors[.]txt$#URIs: https://mirrors.edge.kernel.org/ubuntu/#' \
        "$sources" > "$fallback"
    cmp -s "$sources" "$fallback" && exit "$status"
    chmod 644 "$fallback"
    sources=$fallback
    echo "Retrying Ubuntu package downloads from the kernel.org archive mirror" >&2
    deadline=0
    fetch "$@"
}

# Configure once, offline. A package's maintainer scripts can legitimately take time.
deadline=0
apt install --no-download -y -qq "$@"
