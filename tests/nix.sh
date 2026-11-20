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
app() {  # hyperfine from the default nixpkgs or a pinned branch
    source=
    if [ -n "${1:-}" ]; then source=", nixpkgs: \"github:NixOS/nixpkgs/$1\""; fi
    printf 'metadata: { furnish: { description: Benchmarking, categories: [Test] } }\nresources:\n  - name: hyperfine\n    type: Furnish/Package\n    properties: { name: hyperfine, packageManager: nix%s }\n' "$source" \
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
furnish uninstall hyperfine >/dev/null || fail "uninstall failed"
if profile | grep -q hyperfine; then fail "still in the shared profile after uninstall"; fi
echo "== no module or package source override: an ordinary package still uses the profile"
nix --extra-experimental-features "nix-command flakes" registry add nixpkgs github:NixOS/nixpkgs/nixos-25.11
app
git commit -qam 'use the default nixpkgs'
furnish collection update org >/dev/null
furnish install hyperfine >/dev/null || fail "default Nix package install failed"
profile | grep -q hyperfine || fail "default Nix package missing from profile"
furnish uninstall hyperfine >/dev/null || fail "default Nix package uninstall failed"
if profile | grep -q hyperfine; then fail "default Nix package remains after uninstall"; fi
echo "== a collection-local flake tracks source changes at the same package version"
mkdir -p "files/nix/local package"
cat >"files/nix/local package/flake.nix" <<'FLAKE'
{
  inputs.nixpkgs.url = "nixpkgs";
  outputs = { self, nixpkgs }: {
    packages.x86_64-linux.local-flake-demo = nixpkgs.legacyPackages.x86_64-linux.runCommand
      "local-flake-demo-1.0" { version = "1.0"; }
      "mkdir -p $out/share; cp ${./payload.rpm} $out/share/local-flake-payload";
  };
}
FLAKE
printf 'one' > "files/nix/local package/payload.rpm"
(cd "files/nix/local package" && nix --extra-experimental-features 'nix-command flakes' flake lock path:.)
cat >apps/local.dsc.yaml <<'YAML'
resources:
  - name: Local package
    type: Furnish/Package
    properties:
      packageManager: nix
      name: local-flake-demo
      flake: files/nix/local package
YAML
git add -A && git commit -qm 'ship a local flake'
furnish collection update org >/dev/null
furnish install local >/dev/null
payload=/nix/var/nix/profiles/furnish/share/local-flake-payload
[ "$(cat "$payload")" = one ] || fail "local package missing"
furnish list | grep -q "✓  local" || fail "installed local flake is stale"
printf 'two' > "files/nix/local package/payload.rpm"
git add -A && git commit -qm 'update only the package source'
furnish collection update org >/dev/null
furnish list | grep -q "↑  local" || fail "changed flake with the same version was missed"
furnish install --dry-run local | grep -q "changed collection flake" || fail "missing source update preview"
furnish install local >/dev/null
[ "$(cat "$payload")" = two ] || fail "updated local package still has old files"
furnish list | grep -q "✓  local" || fail "update did not converge"
# A new folder hash may describe a flake that fails to evaluate. Keep the working package.
printf 'not a flake' > "files/nix/local package/flake.nix"
git add -A && git commit -qm 'broken local flake'
furnish collection update org >/dev/null
furnish install local >/dev/null 2>&1 || true
[ "$(cat "$payload")" = two ] || fail "failed update removed the working profile entry"
furnish uninstall local >/dev/null
[ ! -f "$payload" ] || fail "local package remains in the profile"
echo "nix OK"
