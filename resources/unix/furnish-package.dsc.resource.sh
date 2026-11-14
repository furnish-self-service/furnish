#!/bin/sh
# Furnish/Package: manage a package with the system package manager.
# DSC passes the resource properties as env vars: $name and (optionally) $_exist.
set -eu
. "$(dirname "$0")/furnish-common.sh"

[ -n "${name:-}" ] || fail 10 "property name is required"
identity=$(json_member name "$name")
subject=$name

# rpm -q prints "package X is not installed" on stdout, so only print on success.
rpm_version() { v=$(rpm -q --qf '%{VERSION}-%{RELEASE}' "$name" 2>/dev/null) && echo "$v"; }

# $name is restricted to [A-Za-z0-9@._+-] by the schema.
admin=true
if command -v pacman >/dev/null 2>&1; then
    manager=pacman
    installed_version() { pacman -Q "$name" 2>/dev/null | cut -d' ' -f2; }
    do_install() { run_step pacman -S --noconfirm --needed "$name"; }
    do_remove() { run_step pacman -R --noconfirm "$name"; }
    install_desc="pacman -S --noconfirm --needed $name" remove_desc="pacman -R --noconfirm $name"
elif command -v apt-get >/dev/null 2>&1; then
    manager=apt
    installed_version() { dpkg-query -W -f='${db:Status-Status} ${Version}' "$name" 2>/dev/null | sed -n 's/^installed //p'; }
    do_install() { run_step env DEBIAN_FRONTEND=noninteractive apt-get install -y "$name"; }
    do_remove() { run_step env DEBIAN_FRONTEND=noninteractive apt-get remove -y "$name"; }
    install_desc="apt-get install -y $name" remove_desc="apt-get remove -y $name"
elif command -v dnf >/dev/null 2>&1; then
    manager=dnf
    installed_version() { rpm_version; }
    do_install() { run_step dnf install -y "$name"; }
    do_remove() { run_step dnf remove -y "$name"; }
    install_desc="dnf install -y $name" remove_desc="dnf remove -y $name"
elif command -v zypper >/dev/null 2>&1; then
    manager=zypper
    installed_version() { rpm_version; }
    do_install() { run_step zypper --non-interactive install "$name"; }
    do_remove() { run_step zypper --non-interactive remove "$name"; }
    install_desc="zypper --non-interactive install $name" remove_desc="zypper --non-interactive remove $name"
elif command -v brew >/dev/null 2>&1; then
    manager=Homebrew
    admin=false # Homebrew refuses to run as root
    installed_version() { brew list --versions "$name" 2>/dev/null | cut -d' ' -f2; }
    do_install() { run_step brew install "$name"; }
    do_remove() { run_step brew uninstall "$name"; }
    install_desc="brew install $name" remove_desc="brew uninstall $name"
elif command -v nix >/dev/null 2>&1; then
    # NixOS (or nix-only systems): per-user profile from nixpkgs, needs no root.
    manager=nix
    admin=false
    export NIX_CONFIG="experimental-features = nix-command flakes"
    installed_version() {
        nix profile list --json 2>/dev/null | tr '{,' '\n\n' | grep -q "^\"$name\":" || return 0
        path=$(nix profile list --json | grep -o "/nix/store/[a-z0-9]*-$name-[^\"]*" | head -n1)
        echo "${path##*-"$name"-}"
    }
    do_install() { run_step nix profile install "nixpkgs#$name"; }
    do_remove() { run_step nix profile remove "$name"; }
    install_desc="nix profile install nixpkgs#$name" remove_desc="nix profile remove $name"
else
    fail 11 "no supported package manager found (pacman, apt-get, dnf, zypper, brew, nix)"
fi

furnish_main "$@"
