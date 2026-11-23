#!/bin/sh
# Run the CLI from the mounted disk image with only Apple's system tools on PATH.
set -eu
bin=${1:?usage: bundled-lfs-macos.sh <Contents/MacOS>}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export HOME="$tmp/home" FURNISH_DATA_DIR="$tmp/data" FURNISH_SYSTEM_DIR="$tmp/system"
mkdir -p "$HOME"
repo="$tmp/collection"
git init -q "$repo"
PATH="$bin:/usr/bin:/bin" git -C "$repo" lfs install --local
printf 'payload.bin filter=lfs diff=lfs merge=lfs -text\n' > "$repo/.gitattributes"
printf 'expected LFS payload\n' > "$repo/payload.bin"
PATH="$bin:/usr/bin:/bin" git -C "$repo" add .
PATH="$bin:/usr/bin:/bin" git -C "$repo" -c user.name=test -c user.email=test@example.org commit -qm fixture
git -C "$repo" show HEAD:payload.bin | grep -q '^oid sha256:'
PATH=/usr/bin:/bin "$bin/furnish" collection add "file://$repo" --name fixture
cmp "$repo/payload.bin" "$FURNISH_DATA_DIR/collections/fixture/payload.bin"
