#!/bin/sh
# Furnish/Package with Nix on a NixOS-like system: packages from the NixOS configuration
# (/run/current-system/sw) and from Home Manager (inside /etc/profiles/per-user/<user>) show as
# installed, with where they come from; furnish doesn't install them again or remove them.
# The symlink farms are fakes with the names and store references NixOS's buildEnv gives them.
#
#   tests/nix-declared.sh <path/to/static/furnish>
# With GITHUB_TOKEN set (CI), Nix uses it for github: flake references (see tests/nix.sh).
set -eu
if [ "${1:-}" != --inside ]; then
    binary=$(realpath "${1:?usage: $0 <furnish-binary>}")
    exec docker run --rm ${GITHUB_TOKEN:+-e "NIX_CONFIG=access-tokens = github.com=$GITHUB_TOKEN"} \
        -v "$binary:/usr/local/bin/furnish:ro" -v "$(realpath "$0"):/t.sh:ro" nixos/nix sh /t.sh --inside
fi
fail() { echo "FAIL: $*" >&2; exit 1; }
export FURNISH_DATA_DIR=/tmp/data PATH="/usr/local/bin:$PATH"
nixpkgs=github:NixOS/nixpkgs/nixos-25.11
command -v git >/dev/null || nix-env -iA nixpkgs.git >/dev/null 2>&1
git config --global user.name t; git config --global user.email t@t; git config --global init.defaultBranch main

echo "== fake NixOS: system-path with jq (bin, man) and nodejs; Home Manager's hello in the user's farm"
cat >/tmp/farm.nix <<'NIX'
let
  system = builtins.currentSystem;
  build = builtins.toFile "build.sh" ''
    for o in $outputs; do eval "p=\$$o"; echo x > "$p"; done
  '';
  pkg = name: outputs: derivation { inherit name system outputs; builder = "/bin/sh"; args = [ build ]; };
  farm = name: paths: derivation { inherit name system; builder = "/bin/sh"; args = [ "-c" "echo ${toString paths} > $out" ]; };
  jq = pkg "jq-1.7.1" [ "bin" "man" ];
in {
  system = farm "system-path" [ jq.bin jq.man (pkg "nodejs-22.12.0" [ "out" ]) ];
  user = farm "user-environment" [ (farm "home-manager-path" [ (pkg "hello-2.12.1" [ "out" ]) ]) ];
}
NIX
mkdir -p /run/current-system /etc/profiles/per-user
ln -s "$(nix-build /tmp/farm.nix -A system --no-out-link)" /run/current-system/sw
ln -s "$(nix-build /tmp/farm.nix -A user --no-out-link)" "/etc/profiles/per-user/$(id -un)"

mkdir -p /tmp/org/apps
app() {  # id, then extra properties
    printf 'metadata: { furnish: { description: Test, categories: [Test] } }\nresources:\n  - name: pkg\n    type: Furnish/Package\n    properties: { packageManager: nix, nixpkgs: "%s", %s }\n' "$nixpkgs" "$2" \
        >"/tmp/org/apps/$1.dsc.yaml"
}
app jq 'name: jq'
app hello 'name: hello'
# The attribute's last part isn't the package's name: furnish asks nixpkgs (nodejs_22 → nodejs).
app node 'name: node, nixName: nodejs_22'
app ripgrep 'name: ripgrep'
cd /tmp/org && git init -q && git add -A && git commit -qm apps
furnish collection add /tmp/org --name org >/dev/null

echo "== declared packages show as installed, with their source"
list=$(furnish list)
echo "$list" | grep "✓  jq " | grep -q "in your NixOS configuration" || { echo "$list"; fail "jq not shown as from the NixOS configuration"; }
echo "$list" | grep "✓  hello " | grep -q "in your Home Manager configuration" || { echo "$list"; fail "hello not shown as from Home Manager"; }
echo "$list" | grep "✓  node " | grep -q "in your NixOS configuration" || { echo "$list"; fail "node (nodejs_22) not found by its nixpkgs name"; }
echo "$list" | grep -q "·  ripgrep " || { echo "$list"; fail "ripgrep shown as installed"; }
ls "$FURNISH_DATA_DIR"/cache/nix/*nodejs_22 >/dev/null 2>&1 || fail "nixpkgs' name for nodejs_22 not cached"

echo "== the resource says where it comes from"
resource=$FURNISH_DATA_DIR/resources/furnish-package.dsc.resource.sh
out=$(name=jq packageManager=nix sh "$resource" get)
case $out in *'"version":"1.7.1","managedBy":"nixos"'*) ;; *) fail "get: $out" ;; esac
out=$(name=hello packageManager=nix sh "$resource" get)
case $out in *'"managedBy":"home-manager"'*) ;; *) fail "get: $out" ;; esac
rc=0; out=$(_exist=false name=jq packageManager=nix sh "$resource" set 2>&1) || rc=$?
[ "$rc" = 14 ] || fail "removing a declared package: exit $rc, $out"
case $out in *"jq is in your NixOS configuration: remove it there"*) ;; *) fail "removing a declared package: $out" ;; esac

echo "== installing doesn't add a second copy; uninstalling refuses"
furnish install jq | grep -q "already installed: it is in your NixOS configuration" || fail "install jq didn't say it is declared"
! nix --extra-experimental-features "nix-command flakes" profile list --profile /nix/var/nix/profiles/furnish 2>/dev/null | grep -q jq \
    || fail "jq added to the shared profile"
rc=0; out=$(furnish uninstall jq 2>&1) || rc=$?
[ "$rc" != 0 ] || fail "uninstalling a declared package succeeded"
case $out in *"in your NixOS configuration: remove jq there"*) ;; *) fail "uninstall: $out" ;; esac

echo "== without the roots (FURNISH_NIX_ROOTS, for tests only), nothing is declared"
FURNISH_NIX_ROOTS= furnish list | grep -q "·  jq " || fail "jq still installed without the roots"
echo "nix-declared OK"
