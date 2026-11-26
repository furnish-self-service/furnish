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
app icondark "metadata: { furnish: { description: Dark icon, categories: [Test], iconDark: icons/missing-dark.svg } }
$pkg"
app recommended "metadata: { furnish: { description: Recommended, categories: [Test], recommended: \"[equals(parameters('nope'), 'x')]\" } }
$pkg"
app empty 'metadata: { furnish: { description: Empty, categories: [Test] } }
resources: []'
app undescribed "$pkg"
# DSC matches types ignoring case: a lower-case type gets the same checks.
app lowercase 'metadata: { furnish: { description: Lower case, categories: [Test] } }
resources:
  - name: setup
    type: furnish/installer
    properties: { file: installers/lower.msi }'
# Configuration resources: a line that doesn't match its pattern, a source file that isn't there,
# and a command that can't be removed.
# The checker validates vendor download sources offline, including Windows property casing.
pin=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
vendor_app() {
    app "$1" "metadata: { furnish: { description: Vendor package, categories: [Test] } }
resources:
  - name: setup
    type: Furnish/Installer
    properties: { $2 }"
}
vendor_app download "url: https://example.invalid/vendor.pkg, sha256: '$pin', packageId: org.example.vendor"
vendor_app unpinned "url: https://example.invalid/vendor.pkg"
vendor_app mixed "file: vendor.pkg, url: https://example.invalid/vendor.pkg, sha256: '$pin'"
vendor_app wrongcase "URL: https://example.invalid/vendor.pkg, sha256: '$pin'"
app line 'metadata: { furnish: { description: Line, categories: [Test] } }
resources:
  - name: editor
    type: Furnish/Line
    properties: { path: ~/.profile, line: EDITOR=vim, match: "^export EDITOR=" }'
app source 'metadata: { furnish: { description: Source, categories: [Test] } }
resources:
  - name: profile
    type: Furnish/File
    properties: { path: ~/.acme/profile.conf, source: files/missing.conf }'
app command 'metadata: { furnish: { description: Command, categories: [Test] } }
resources:
  - name: server
    type: Furnish/Command
    properties: { test: { command: [bw, config, server], output: "https://vault.acme.example" }, set: [bw, config, server, "https://vault.acme.example"] }'
# A command that uninstalling leaves in place (remove: keep) is removable; another word is a mistake.
app kept 'metadata: { furnish: { description: Kept, categories: [Test] } }
resources:
  - name: marketplace
    type: Furnish/Command
    properties: { test: { command: [assistant, plugin, marketplace, list] }, set: [assistant, plugin, marketplace, add, acme], remove: keep }'
app leave 'metadata: { furnish: { description: Leave, categories: [Test] } }
resources:
  - name: marketplace
    type: Furnish/Command
    properties: { test: { command: [assistant, plugin, marketplace, list] }, set: [assistant, plugin, marketplace, add, acme], remove: leave }'
# An icon that links out of the collection.
echo '<svg/>' > "$tmp/outside.svg"
ln -s "$tmp/outside.svg" "$c/icons/outside.svg"
cat > "$c/collection.yaml" <<'YAML'
icon: icons/outside.svg
iconDark: icons/missing-dark.svg
parameters:
  team: { type: string, required: true }
YAML
printf 'description: Kit\nitems:\n  - app: good\n  - app: ghost\n' > "$c/kits/starter.yaml"
cat > "$c/kits/team.yaml" <<'YAML'
recommended: "[equals(parameters('team'), 'engineering')]"
items: [{ app: good }]
YAML
cat > "$c/kits/dark.yaml" <<'YAML'
description: Kit
iconDark: icons/missing-dark.svg
recommended: "[parameters('unknown')]"
items:
  - app: good
YAML
git -C "$c" init -q && git -C "$c" add -A && git -C "$c" -c user.name=t -c user.email=t@t commit -qm init
# The checked repository's config must not run commands (checking an installer runs git there).
git -C "$c" config core.fsmonitor "touch '$tmp/pwned'"

out=$("$furnish" check "$c" 2>&1); code=$?
[ $code -ne 0 ] && ok "a collection with errors fails (exit $code)" || bad "exit 0 despite errors"
expect() { # expect <app> <text>: the app's block names the mistake
    if echo "$out" | awk -v app="$1" '/^[^ ]/ {on = ($2 == app); next} on' | grep -qF -- "$2"; then ok "$1: $2"
    else bad "$1: no \"$2\" in: $(echo "$out" | grep -A3 " $1 ")"; fi
}
echo "$out" | grep -q '^✓ good ' && ok "good: no findings" || bad "good: $(echo "$out" | grep -A2 ' good ')"
expect yaml "error:"
expect installer "installers/missing.msi is not in the collection"
echo "$out" | grep -q '^✓ download ' && ok "valid pinned download checks offline" || bad "download: $(echo "$out" | grep -A3 ' download ')"
expect unpinned "either file or both url and sha256"
expect mixed "either file or both url and sha256"
expect wrongcase "did you mean 'url'"
expect noname "needs properties.name"
expect depends "depends on ghost, which is not an app"
expect cycle-a "dependencies form a cycle"
expect condition "condition"
expect icon "icon 'icons/missing.svg' is not in the collection"
expect icondark "iconDark 'icons/missing-dark.svg' is not in the collection"
expect dark "iconDark 'icons/missing-dark.svg' is not in the collection"
expect recommended "recommended:"
expect dark "recommended:"
echo "$out" | grep -q '^✓ team ' && ok "kit with unanswered question" || bad "kit with unanswered question"
expect empty "no resources"
expect undescribed "warning: no description"
expect starter "ghost"
expect lowercase "installers/lower.msi is not in the collection"
expect line "doesn't match"
expect source "files/missing.conf is not in the collection"
expect command "can't be uninstalled where these resources apply: server (Furnish/Command)"
echo "$out" | grep -q '^✓ kept ' && ok "kept: remove: keep is removable" || bad "kept: $(echo "$out" | grep -A2 ' kept ')"
expect leave "or 'keep' to leave the setting in place"
expect collection.yaml "icon 'icons/outside.svg' is not in the collection"
expect collection.yaml "iconDark 'icons/missing-dark.svg' is not in the collection"
[ ! -e "$tmp/pwned" ] && ok "the repository's core.fsmonitor did not run" || bad "core.fsmonitor ran"
echo "$out" | grep -qE '^[0-9]+ apps, 3 kits: [0-9]+ errors, [0-9]+ warnings?$' && ok "summary line" || bad "no summary: $(echo "$out" | tail -1)"

out=$("$furnish" check "$fixture" 2>&1) && ok "the test fixture passes" || bad "fixture: $out"
echo "$out" | grep -q '^✓ hyperfine .*Windows: hyperfine (winget)' && ok "shows what runs on each system" || bad "no per-system preview: $out"
cat > "$tmp/kit-profile.yaml" <<'YAML'
profiles:
  kit-test:
    family: Linux
    packageManagers: [pacman]
    expectKits:
      starter:
        applicable: []
        required: []
        defaultSelection: []
YAML
out=$("$furnish" check "$fixture" --profiles "$tmp/kit-profile.yaml" 2>&1)
[ $? -ne 0 ] && ok "incorrect kit expectations fail" || bad "kit assertion unexpectedly passed"
for field in applicable required defaultSelection; do
    expect kit-test "starter: expected $field []"
done
exit $failed
