#!/bin/sh
# furnish check: a collection with one mistake per app must fail with each mistake named; the test
# fixture must pass.
#
#   tests/check.sh [path/to/furnish]
set -u
furnish=$(realpath "${1:-target/fast/furnish}")
fixture=$(realpath "$(dirname "$0")/fixtures/collection")
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export FURNISH_DATA_DIR="$tmp/data"
failed=0
ok()  { echo "ok   $*"; }
bad() { echo "FAIL $*"; failed=1; }

c=$tmp/broken
mkdir -p "$c/apps" "$c/kits" "$c/icons"
app() { printf '%s\n' "$2" > "$c/apps/$1.dsc.yaml"; }
pkg='resources:
  - name: tool
    type: Furnish/Package
    properties: { name: tool }'
app good "metadata: { furnish: { description: Fine, categories: [Test] } }
$pkg"
app yaml "metadata: { furnish: { description: [unclosed }
$pkg"
app installer 'metadata: { furnish: { description: Installer, categories: [Test] } }
resources:
  - name: setup
    type: Furnish/Installer
    properties: { file: installers/missing.msi }'
app noname 'metadata: { furnish: { description: No name, categories: [Test] } }
resources:
  - name: tool
    type: Furnish/Package
    properties: {}'
app depends "metadata: { furnish: { description: Depends, categories: [Test], dependencies: [{app: ghost}] } }
$pkg"
app cycle-a "metadata: { furnish: { description: A, categories: [Test], dependencies: [{app: cycle-b, required: true}] } }
$pkg"
app cycle-b "metadata: { furnish: { description: B, categories: [Test], dependencies: [{app: cycle-a, required: true}] } }
$pkg"
app condition 'metadata: { furnish: { description: Condition, categories: [Test] } }
resources:
  - name: tool
    type: Furnish/Package
    condition: "[equals(context().os.family, 'Windows'"
    properties: { name: tool }'
app icon "metadata: { furnish: { description: Icon, categories: [Test], icon: icons/missing.svg } }
$pkg"
app recommended "metadata: { furnish: { description: Recommended, categories: [Test], recommended: \"[equals(parameters('nope'), 'x')]\" } }
$pkg"
app empty 'metadata: { furnish: { description: Empty, categories: [Test] } }
resources: []'
app undescribed "$pkg"
printf 'description: Kit\nitems:\n  - app: good\n  - app: ghost\n' > "$c/kits/starter.yaml"
git -C "$c" init -q && git -C "$c" add -A && git -C "$c" -c user.name=t -c user.email=t@t commit -qm init

out=$("$furnish" check "$c" 2>&1); code=$?
[ $code -ne 0 ] && ok "a collection with errors fails (exit $code)" || bad "exit 0 despite errors"
expect() { # expect <app> <text>: the app's block names the mistake
    if echo "$out" | awk -v app="$1" '/^[^ ]/ {on = ($2 == app); next} on' | grep -qF -- "$2"; then ok "$1: $2"
    else bad "$1: no \"$2\" in: $(echo "$out" | grep -A3 " $1 ")"; fi
}
echo "$out" | grep -q '^✓ good ' && ok "good: no findings" || bad "good: $(echo "$out" | grep -A2 ' good ')"
expect yaml "error:"
expect installer "installers/missing.msi is not in the collection"
expect noname "needs properties.name"
expect depends "depends on ghost, which is not an app"
expect cycle-a "dependencies form a cycle"
expect condition "condition"
expect icon "icon 'icons/missing.svg' is not in the collection"
expect recommended "recommended:"
expect empty "no resources"
expect undescribed "warning: no description"
expect starter "ghost"
echo "$out" | grep -qE '^[0-9]+ apps, 1 kit: [0-9]+ errors, [0-9]+ warnings?$' && ok "summary line" || bad "no summary: $(echo "$out" | tail -1)"

out=$("$furnish" check "$fixture" 2>&1) && ok "the test fixture passes" || bad "fixture: $out"
echo "$out" | grep -q '^✓ hyperfine .*Windows: hyperfine (winget)' && ok "shows what runs on each system" || bad "no per-system preview: $out"
exit $failed
