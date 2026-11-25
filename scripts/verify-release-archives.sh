#!/bin/sh
# Both release channels must contain each expected archive under its public name.
set -eu
dir=${1:?release assets directory required}
version=${2:?release version required}
for suffix in windows-x86_64_user-setup.exe windows-x86_64_binaries.zip arm64.app.tar.gz macos-arm64_binaries.tar.gz linux-x86_64_binaries.tar.gz linux-x86_64_user.tar.gz; do
    file="$dir/furnish_${version}_$suffix"
    [ -s "$file" ] || { echo "missing release asset: $file" >&2; exit 1; }
done
