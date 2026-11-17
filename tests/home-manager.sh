#!/bin/sh
# Standalone Home Manager on a Linux system that isn't NixOS (the nixos/nix image): furnish shows
# a Home Manager section in `furnish settings` and creates its local flake; then it installs an app
# into the configuration (build and activation), an ssh Include through Home Manager that a
# Furnish/Line step accepts, and uninstalls the app again. As root, without the service.
#
#   tests/home-manager.sh <path/to/static/furnish>
# With GITHUB_TOKEN set (CI), Nix uses it for github: flake references.
set -eu
if [ "${1:-}" != --inside ]; then
    binary=$(realpath "${1:?usage: $0 <furnish-binary>}")
    root=$(cd "$(dirname "$0")/.." && pwd)
    # furnish's flake as a checkout has it, without build output.
    src=$(mktemp -d)
    trap 'rm -rf "$src"' EXIT
    (cd "$root" && git ls-files -z --cached --others --exclude-standard | xargs -0 tar -cf -) | tar -xf - -C "$src"
    docker run --rm ${GITHUB_TOKEN:+-e "NIX_CONFIG=access-tokens = github.com=$GITHUB_TOKEN"} \
        -v "$binary:/usr/local/bin/furnish:ro" -v "$src:/src:ro" -v "$root/tests/home-manager.sh:/t.sh:ro" \
        nixos/nix sh /t.sh --inside
    exit
fi
fail() {
    echo "FAIL: $*" >&2
    echo "-- furnish's Home Manager facts, the generation, furnish's flake:" >&2
    ls -la /root/.config/furnish /root/.local/share/furnish/home-apps 2>&1 | head -20 >&2
    cat /root/.config/furnish/home-manager.json /root/.local/share/furnish/home-apps/apps.json 2>&1 | head -20 >&2
    readlink -f /root/.local/state/home-manager/gcroots/current-home >&2 || true
    exit 1
}
export PATH="/usr/local/bin:$PATH" USER=root HOME=/root
nix() { command nix --extra-experimental-features "nix-command flakes" "$@"; }
[ ! -e /etc/NIXOS ] || fail "this is NixOS"
command -v git >/dev/null || nix-env -iA nixpkgs.git >/dev/null 2>&1
git config --global user.name t; git config --global user.email t@t; git config --global init.defaultBranch main
# The active generation, wherever Home Manager keeps its profile.
generation() { readlink -f /root/.local/state/home-manager/gcroots/current-home; }

echo "== the organization's apps"
mkdir -p /tmp/org/apps
cat > /tmp/org/apps/htop.dsc.yaml <<'EOF'
metadata: { furnish: { description: htop, categories: [Test] } }
resources:
  - name: htop
    type: Furnish/NixOS
    properties: { name: htop, packages: [hello], homeOptions: { programs.htop.enable: true } }
EOF
cat > /tmp/org/apps/acme-ssh.dsc.yaml <<'EOF'
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
(cd /tmp/org && git init -q && git add -A && git commit -qm apps)
furnish collection add /tmp/org --name org >/dev/null

echo "== a Home Manager configuration without furnish's module: Settings shows the lines"
cfg=/root/.config/home-manager
mkdir -p "$cfg"
cat > "$cfg/flake.nix" <<'EOF'
{
  inputs.furnish.url = "path:/src";
  inputs.furnish.inputs.home-apps.url = "path:/root/.local/share/furnish/home-apps";
  inputs.home-manager.url = "github:nix-community/home-manager/3ee51fbdac8c8bdfe1e7e1fcaba6520a563f394f";
  inputs.home-manager.inputs.nixpkgs.follows = "furnish/nixpkgs";

  outputs = { furnish, home-manager, ... }: {
    homeConfigurations.root = home-manager.lib.homeManagerConfiguration {
      pkgs = furnish.inputs.nixpkgs.legacyPackages.x86_64-linux;
      modules = [ furnish.homeManagerModules.default ./home.nix ];
    };
  };
}
EOF
cat > "$cfg/home.nix" <<'EOF'
{
  home.username = "root";
  home.homeDirectory = "/root";
  home.stateVersion = "25.11";
  manual.manpages.enable = false;
  programs.ssh.enable = true;
  programs.ssh.includes = [ "~/.ssh/own.conf" ];
}
EOF
out=$(furnish settings)
echo "$out" | grep -q '^Home Manager: *'"$cfg" || { echo "$out"; fail "no Home Manager line"; }
echo "$out" | grep -q 'NixOS configuration' && { echo "$out"; fail "a NixOS line without NixOS"; }
[ -f /root/.local/share/furnish/home-apps/apps.json ] || fail "furnish didn't create its local flake"

echo "== the module on"
cat > "$cfg/home.nix" <<'EOF'
{
  home.username = "root";
  home.homeDirectory = "/root";
  home.stateVersion = "25.11";
  manual.manpages.enable = false;
  programs.ssh.enable = true;
  programs.ssh.includes = [ "~/.ssh/own.conf" ];
  services.furnish.enable = true;
  services.furnish.nix.allowedOptions = [ "programs.htop.enable" "programs.ssh.includes" ];
}
EOF
cd "$cfg"
git init -q && git add -A
nix flake lock 2>&1 | tail -3
git add -A && git commit -qm config
nix build -L .#homeConfigurations.root.activationPackage 2>&1 | tail -3
# The image's root profile is its whole base system; a person's profile holds only their own
# packages, so Home Manager starts with its own (the base system stays on PATH).
rm /root/.nix-profile
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
[ -L /root/.ssh/config ] || fail "Home Manager doesn't manage ~/.ssh/config"
grep -E '^Include .*config\.d/acme\.conf' /root/.ssh/config | grep -q own.conf || { cat /root/.ssh/config; fail "no combined Include"; }
furnish list | grep -q "✓  acme-ssh" || fail "acme-ssh not shown as installed"

echo "== uninstall"
furnish uninstall htop || fail "uninstall failed"
test -e "$(generation)/home-path/bin/hello" && fail "hello is still there"
[ "$(git log -1 --format=%s)" = "furnish: uninstall htop" ] || fail "flake.lock not committed"
echo "OK"
