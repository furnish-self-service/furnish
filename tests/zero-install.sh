#!/usr/bin/env bash
# Exercise Furnish against the real Unix Zero Install client, entirely offline.
set -euo pipefail
furnish=$(realpath "${1:-target/fast/furnish}")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
export HOME="$work/home" XDG_CONFIG_HOME="$work/config" XDG_CONFIG_DIRS="$work/system-config"
export XDG_CACHE_HOME="$work/cache" XDG_DATA_HOME="$work/share" FURNISH_DATA_DIR="$work/furnish"
unset ZEROINSTALL_PORTABLE_BASE
mkdir -p "$HOME/bin" "$work/feed" "$work/collection/apps"
export PATH="$HOME/bin:$PATH"
feed=https://example.test/furnish-hello.xml
cat > "$work/feed/feed.xml" <<XML
<interface xmlns="http://zero-install.sourceforge.net/2004/injector/interface">
  <name>Hello</name><summary>Furnish fixture</summary>
  <feed-for interface="$feed"/>
  <implementation id="." version="1.0" main="hello"/>
</interface>
XML
printf '#!/bin/sh\necho hello-from-zero-install\n' > "$work/feed/hello"
chmod +x "$work/feed/hello"
0install config network_use off-line
0install add-feed --offline "$feed" "$work/feed/feed.xml"

cat > "$work/collection/apps/hello.dsc.yaml" <<YAML
metadata: { furnish: { description: Test the real Zero Install client, categories: [Test] } }
resources:
  - name: Install the greeting
    type: Furnish/Package
    properties: { name: hello, namesByManager: { 0install: furnish-hello, pacman: native-greeting }, feed: $feed }
YAML
git -C "$work/collection" init -q
git -C "$work/collection" add .
git -C "$work/collection" -c user.name=fixture -c user.email=fixture@example.test commit -qm fixture
"$furnish" check "$work/collection"
"$furnish" collection add "$work/collection" --name fixture
"$furnish" install hello --dry-run | grep -F '0install add'
test ! -e "$HOME/bin/furnish-hello"
"$furnish" install hello
test "$(furnish-hello)" = hello-from-zero-install
receipt=$(find "$FURNISH_DATA_DIR/package-owners" -type f -print -quit)
test -n "$receipt"
# Without the recorded client the package resource must report the prerequisite,
# even though the package could otherwise fall back to another manager or command.
appdir=${receipt%/*}
key="package:${appdir##*/}:${receipt##*/}"
mkdir "$work/empty-path"
printf '{"name":"hello","_metadata":{"furnishNixPackage":{"key":"%s"}}}' "$key" |
    PATH="$work/empty-path" "$furnish" dsc-resource package test >"$work/missing" 2>&1 && exit 1
grep -F 'needs package manager 0install' "$work/missing"
# A newly available, higher-priority manager must never see a status or removal request.
printf '#!/bin/sh\necho unexpected-pacman >> "%s"\nexit 99\n' "$work/wrong-manager" > "$HOME/bin/pacman"
chmod +x "$HOME/bin/pacman"
"$furnish" install hello
test "$(0install show --root-uri furnish-hello)" = "$feed"
# A package that another installer provides stays under its original ownership.
printf '%s' '{"packageManager":"0install","name":"external-tool","feed":"https://example.test/tool.xml","command":"git"}' |
    "$furnish" dsc-resource package get | grep -F '"managedBy":"other"'
# Another collection sees its feed as absent. It cannot overwrite or destroy the other launcher.
foreign='{"packageManager":"0install","feed":"https://example.test/other.xml","name":"furnish-hello"}'
printf '%s' "$foreign" | "$furnish" dsc-resource package test | grep -F '"_inDesiredState":false'
if printf '%s' "$foreign" | "$furnish" dsc-resource package set >"$work/out" 2>&1; then
    echo 'Overwrote a different feed' >&2; exit 1
fi
grep -F 'belongs to' "$work/out"
printf '%s' '{"packageManager":"0install","feed":"https://example.test/other.xml","name":"furnish-hello","command":"furnish-hello","_exist":false}' |
    "$furnish" dsc-resource package set | grep -F '"_exist":false'
test "$(furnish-hello)" = hello-from-zero-install
"$furnish" uninstall hello --dry-run | grep -F '0install destroy'
test "$(furnish-hello)" = hello-from-zero-install
"$furnish" uninstall hello
test ! -e "$HOME/bin/furnish-hello"
test ! -e "$work/wrong-manager"
test -z "$(find "$FURNISH_DATA_DIR/package-owners" -type f -print -quit)"
