#!/bin/sh
# Standalone Home Manager on a system that isn't NixOS: furnish shows a Home Manager section in
# `furnish settings` and creates its local flake; then it installs an app into the configuration
# (build and activation), an ssh Include through Home Manager that a Furnish/Line step accepts,
# and uninstalls the app again. Without the service.
#
#   tests/home-manager.sh <path/to/static/furnish>         Linux: as root in the nixos/nix image
#   tests/home-manager.sh --here <path/to/furnish>         here, as you, with Nix installed (macOS CI)
# With GITHUB_TOKEN set (CI), Nix uses it for github: flake references.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
case "${1:-}" in
--inside) src=/src ;;
--here)
    furnish=$(realpath "${2:?usage: $0 --here <furnish-binary>}")
    src=$(mktemp -d)
    (cd "$root" && git ls-files -z --cached --others --exclude-standard | xargs -0 tar -cf -) | tar -xf - -C "$src"
    mkdir -p "$src/.bin" && ln -s "$furnish" "$src/.bin/furnish"
    export PATH="$src/.bin:$PATH"
    [ -z "${GITHUB_TOKEN:-}" ] || export NIX_CONFIG="access-tokens = github.com=$GITHUB_TOKEN"
    ;;
*)
    binary=$(realpath "${1:?usage: $0 <furnish-binary>}")
    # furnish's flake as a checkout has it, without build output.
    src=$(mktemp -d)
    trap 'rm -rf "$src"' EXIT
    (cd "$root" && git ls-files -z --cached --others --exclude-standard | xargs -0 tar -cf -) | tar -xf - -C "$src"
    docker run --rm ${GITHUB_TOKEN:+-e "NIX_CONFIG=access-tokens = github.com=$GITHUB_TOKEN"} \
        -v "$binary:/usr/local/bin/furnish:ro" -v "$src:/src:ro" -v "$root/tests/home-manager.sh:/t.sh:ro" \
        nixos/nix sh /t.sh --inside
    exit
    ;;
esac
user=$(id -un)
export USER="$user" PATH="/usr/local/bin:$PATH"
data="$HOME/.local/share/furnish/home-apps"
cfg="$HOME/.config/home-manager"
fail() {
    echo "FAIL: $*" >&2
    echo "-- furnish's Home Manager facts, its flake, the generation:" >&2
    ls -la "$HOME/.config/furnish" "$data" 2>&1 | head -20 >&2
    cat "$HOME/.config/furnish/home-manager.json" "$data/apps.json" 2>&1 | head -20 >&2
    readlink "$HOME/.local/state/home-manager/gcroots/current-home" >&2 || true
    exit 1
}
nix() { command nix --extra-experimental-features "nix-command flakes" "$@"; }
[ ! -e /etc/NIXOS ] || fail "this is NixOS"
command -v git >/dev/null || nix-env -iA nixpkgs.git >/dev/null 2>&1
git config --global user.name t; git config --global user.email t@t; git config --global init.defaultBranch main
system=$(nix eval --impure --raw --expr builtins.currentSystem)
# The active generation, wherever Home Manager keeps its profile.
generation() { readlink "$HOME/.local/state/home-manager/gcroots/current-home"; }
echo "== $system, as $user"

echo "== the organization's apps"
org=$(mktemp -d)
mkdir -p "$org/apps"
cat > "$org/apps/htop.dsc.yaml" <<'EOF'
metadata: { furnish: { description: htop, categories: [Test] } }
resources:
  - name: htop
    type: Furnish/NixOS
    properties: { name: htop, packages: [hello], homeOptions: { programs.htop.enable: true } }
EOF
cat > "$org/apps/acme-ssh.dsc.yaml" <<'EOF'
metadata: { furnish: { description: acme-ssh, categories: [Test] } }
resources:
  - name: SSH include through Home Manager
    type: Furnish/NixOS
    properties: { name: acme-ssh, onlyIfSetUp: true, homeOptions: { programs.ssh.includes: [config.d/acme.conf] } }
  - name: SSH include
    type: Furnish/Line
    properties:
      path: ~/.ssh/config
      line: Include config.d/acme.conf
      before: '^\s*(Host|Match)\s'
      accept: '^\s*Include\s(.*\s)?config\.d/acme\.conf(\s|$)'
EOF
(cd "$org" && git init -q && git add -A && git commit -qm apps)
furnish collection add "$org" --name org >/dev/null

echo "== a Home Manager configuration without furnish's module: Settings shows the lines"
mkdir -p "$cfg"
cat > "$cfg/flake.nix" <<EOF
{
  inputs.furnish.url = "path:$src";
  inputs.furnish.inputs.home-apps.url = "path:$data";
  inputs.home-manager.url = "github:nix-community/home-manager/3ee51fbdac8c8bdfe1e7e1fcaba6520a563f394f";
  inputs.home-manager.inputs.nixpkgs.follows = "furnish/nixpkgs";

  outputs = { furnish, home-manager, ... }: {
    homeConfigurations."$user" = home-manager.lib.homeManagerConfiguration {
      pkgs = furnish.inputs.nixpkgs.legacyPackages.$system;
      modules = [ furnish.homeManagerModules.default ./home.nix ];
    };
  };
}
EOF
home() {
    cat > "$cfg/home.nix" <<EOF
{
  home.username = "$user";
  home.homeDirectory = "$HOME";
  home.stateVersion = "25.11";
  manual.manpages.enable = false;
  programs.ssh.enable = true;
  programs.ssh.includes = [ "~/.ssh/own.conf" ];
  $1
}
EOF
}
home ""
out=$(furnish settings)
echo "$out" | grep -q "^Home Manager: *$cfg" || { echo "$out"; fail "no Home Manager line"; }
echo "$out" | grep -q 'NixOS configuration' && { echo "$out"; fail "a NixOS line without NixOS"; }
echo "$out" | grep -q "home-apps.url = \"path:$data\"" || { echo "$out"; fail "no input line"; }
[ -f "$data/apps.json" ] || fail "furnish didn't create its local flake"

echo "== the module on"
home 'services.furnish.enable = true;
  services.furnish.nix.allowedOptions = [ "programs.htop.enable" "programs.ssh.includes" ];'
cd "$cfg"
git init -q && git add -A
nix flake lock 2>&1 | tail -3
git add -A && git commit -qm config
nix build -L ".#homeConfigurations.\"$user\".activationPackage" 2>&1 | tail -3
# The image's root profile is its whole base system; a person's profile holds only their own
# packages, so Home Manager starts with its own (the base system stays on PATH).
[ "${1:-}" != --inside ] || rm /root/.nix-profile
# Someone's own ssh config would be in Home Manager's way.
[ ! -e "$HOME/.ssh/config" ] || mv "$HOME/.ssh/config" "$HOME/.ssh/config.before-test"
./result/activate > /tmp/activate.log 2>&1 || { tail -30 /tmp/activate.log; fail "activation failed"; }
rm result
out=$(furnish settings)
echo "$out" | grep -q 'home-apps.url' && { echo "$out"; fail "still asks for the input line"; }

echo "== install into Home Manager"
furnish install htop || fail "install failed"
test -x "$(generation)/home-path/bin/hello" || fail "hello isn't in the generation"
test -x "$(generation)/home-path/bin/htop" || fail "htop isn't in the generation"
[ "$(git log -1 --format=%s)" = "furnish: install htop" ] || fail "flake.lock not committed"
[ -z "$(git status --porcelain)" ] || fail "other changes: $(git status --porcelain)"
furnish list | grep -q "✓  htop" || fail "htop not shown as installed"

echo "== the ssh Include through Home Manager, in one line with our own include"
furnish install acme-ssh || fail "install failed"
[ -L "$HOME/.ssh/config" ] || fail "Home Manager doesn't manage ~/.ssh/config"
grep -E '^Include .*config\.d/acme\.conf' "$HOME/.ssh/config" | grep -q own.conf || { cat "$HOME/.ssh/config"; fail "no combined Include"; }
furnish list | grep -q "✓  acme-ssh" || fail "acme-ssh not shown as installed"

echo "== uninstall"
furnish uninstall htop || fail "uninstall failed"
test -e "$(generation)/home-path/bin/hello" && fail "hello is still there"
[ "$(git log -1 --format=%s)" = "furnish: uninstall htop" ] || fail "flake.lock not committed"
echo "OK"
