#!/bin/sh
# Build the package repository with publish.sh from local packages, signed with a throwaway key,
# serve it on 127.0.0.1 and install from it with tests/repo.sh: the publishing logic, on branches.
#
#   tests/repo-local.sh <image> <dir with the .deb, .rpm and .pkg.tar.zst>    e.g. archlinux:latest bundle
#
# Needs what publish.sh needs (apt-ftparchive, createrepo_c, gpg; repo-add or docker), python3, docker.
set -eu
image=${1:?usage: $0 <image> <packages dir>}; packages=${2:?}
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d); server=; trap '[ -z "$server" ] || kill $server || true; rm -rf "$tmp"' EXIT
export GNUPGHOME="$tmp/gnupg"; mkdir -m 700 "$GNUPGHOME"
gpg --batch --quiet --passphrase '' --quick-gen-key 'furnish test <test@example.invalid>' rsa3072 sign never
port=8765
BASE_URL=http://127.0.0.1:$port "$here/../packaging/repo/publish.sh" alpha "$packages" "$tmp/site"
gpg --armor --export > "$tmp/site/gpg-key.asc"   # in place of the real key
python3 -m http.server -b 127.0.0.1 -d "$tmp/site" $port >/dev/null 2>&1 & server=$!
sleep 1
KEY_FPR=$(gpg --with-colons --list-keys | awk -F: '/^fpr/ {print $10; exit}') \
    "$here/repo.sh" "$image" http://127.0.0.1:$port alpha
