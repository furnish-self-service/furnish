#!/bin/sh
# Furnish/Package with Nix, as the furnish service runs it (root): installs into the shared
# profile, shows an update when the collection pins a newer nixpkgs, and installs it.
#
#   tests/nix.sh <path/to/static/furnish>
# With GITHUB_TOKEN set (CI), Nix uses it for github: flake references, which shared runners
# otherwise hit GitHub's API rate limit with; furnish then can't see the newer nixpkgs.
set -eu
if [ "${1:-}" != --inside ]; then
    binary=$(realpath "${1:?usage: $0 <furnish-binary>}")
    exec docker run --rm ${GITHUB_TOKEN:+-e "NIX_CONFIG=access-tokens = github.com=$GITHUB_TOKEN"} \
        -v "$binary:/usr/local/bin/furnish:ro" -v "$(realpath "$0"):/t.sh:ro" nixos/nix sh /t.sh --inside
fi
fail() { echo "FAIL: $*" >&2; exit 1; }
profile() { nix --extra-experimental-features "nix-command flakes" profile list --profile /nix/var/nix/profiles/furnish; }
export FURNISH_DATA_DIR=/tmp/data PATH="/usr/local/bin:$PATH"
command -v git >/dev/null || nix-env -iA nixpkgs.git >/dev/null 2>&1
git config --global user.name t; git config --global user.email t@t; git config --global init.defaultBranch main
mkdir -p /tmp/org/apps
app() {  # hyperfine from a pinned nixpkgs
    printf 'metadata: { furnish: { description: Benchmarking, categories: [Test] } }\nresources:\n  - name: hyperfine\n    type: Furnish/Package\n    properties: { name: hyperfine, packageManager: nix, nixpkgs: "github:NixOS/nixpkgs/%s" }\n' "$1" \
        > /tmp/org/apps/hyperfine.dsc.yaml
}
app nixos-24.11; cd /tmp/org && git init -q && git add -A && git commit -qm old
furnish collection add /tmp/org --name org >/dev/null

echo "== as root: into the shared profile"
furnish install hyperfine >/dev/null || fail "install failed"
profile | grep -q hyperfine || fail "not in the shared profile"
furnish list | grep -q "✓  hyperfine" || fail "not shown as installed"

echo "== the collection pins a newer nixpkgs: an update"
app nixos-25.11; git commit -qam new; furnish collection update org >/dev/null
furnish list | grep -q "↑  hyperfine" || {
    furnish list; furnish collection update org
    echo "nixpkgs nixos-25.11 says:"; nix --extra-experimental-features "nix-command flakes" eval --raw github:NixOS/nixpkgs/nixos-25.11#hyperfine.version 2>&1 | tail -3
    fail "no update shown"; }
furnish install hyperfine >/dev/null || fail "update failed"
furnish list | grep -q "✓  hyperfine" || fail "not up to date after the update"
[ "$(profile | grep -c 'hyperfine-')" = 1 ] || fail "old version left in the profile"
echo "nix OK"
