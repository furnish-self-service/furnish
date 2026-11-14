#!/bin/sh
# Furnish/Package: manage a package with the system package manager.
# DSC passes the resource properties as env vars: $name, $packageManager (optional: pacman, apt,
# dnf, zypper, brew, nix; default: the first one found) and $_exist.
set -eu
. "$(dirname "$0")/furnish-common.sh"

[ -n "${name:-}" ] || fail 10 "property name is required"
identity=$(json_member name "$name")
[ -n "${packageManager:-}" ] && identity="$identity,$(json_member packageManager "$packageManager")"
subject=$name

# rpm -q prints "package X is not installed" on stdout, so only print on success.
rpm_version() { v=$(rpm -q --qf '%{VERSION}-%{RELEASE}' "$name" 2>/dev/null) && echo "$v"; }
have() { command -v "$1" >/dev/null 2>&1; }

# Each backend: returns 1 if its tool is missing, else sets via, admin, the commands and
# installed_version. $name is restricted to [A-Za-z0-9@._+-] by the schema.
use_pacman() {
    have pacman || return 1
    via=pacman admin=true
    installed_version() { pacman -Q "$name" 2>/dev/null | cut -d' ' -f2; }
    do_install() { run_step pacman -S --noconfirm --needed "$name"; }
    do_remove() { run_step pacman -R --noconfirm "$name"; }
    install_desc="pacman -S --noconfirm --needed $name" remove_desc="pacman -R --noconfirm $name"
}
use_apt() {
    have apt-get || return 1
    via=apt admin=true
    installed_version() { dpkg-query -W -f='${db:Status-Status} ${Version}' "$name" 2>/dev/null | sed -n 's/^installed //p'; }
    do_install() { run_step env DEBIAN_FRONTEND=noninteractive apt-get install -y "$name"; }
    do_remove() { run_step env DEBIAN_FRONTEND=noninteractive apt-get remove -y "$name"; }
    install_desc="apt-get install -y $name" remove_desc="apt-get remove -y $name"
}
use_dnf() {
    have dnf || return 1
    via=dnf admin=true
    installed_version() { rpm_version; }
    do_install() { run_step dnf install -y "$name"; }
    do_remove() { run_step dnf remove -y "$name"; }
    install_desc="dnf install -y $name" remove_desc="dnf remove -y $name"
}
use_zypper() {
    have zypper || return 1
    via=zypper admin=true
    installed_version() { rpm_version; }
    do_install() { run_step zypper --non-interactive install "$name"; }
    do_remove() { run_step zypper --non-interactive remove "$name"; }
    install_desc="zypper --non-interactive install $name" remove_desc="zypper --non-interactive remove $name"
}
use_brew() {
    have brew || return 1
    via=Homebrew admin=false # Homebrew refuses to run as root
    installed_version() { brew list --versions "$name" 2>/dev/null | cut -d' ' -f2; }
    do_install() { run_step brew install "$name"; }
    do_remove() { run_step brew uninstall "$name"; }
    install_desc="brew install $name" remove_desc="brew uninstall $name"
}
use_nix() {
    # NixOS (or nix-only systems): per-user profile from nixpkgs, needs no root.
    have nix || return 1
    via=nix admin=false
    export NIX_CONFIG="experimental-features = nix-command flakes"
    installed_version() {
        nix profile list --json 2>/dev/null | tr '{,' '\n\n' | grep -q "^\"$name\":" || return 0
        path=$(nix profile list --json | grep -o "/nix/store/[a-z0-9]*-$name-[^\"]*" | head -n1)
        echo "${path##*-"$name"-}"
    }
    do_install() { run_step nix profile install "nixpkgs#$name"; }
    do_remove() { run_step nix profile remove "$name"; }
    install_desc="nix profile install nixpkgs#$name" remove_desc="nix profile remove $name"
}

case "${packageManager:-}" in
pacman) use_pacman || missing "pacman" ;;
apt) use_apt || missing "apt (Debian, Ubuntu)" ;;
dnf) use_dnf || missing "dnf (Fedora, RHEL)" ;;
zypper) use_zypper || missing "zypper (openSUSE)" ;;
brew) use_brew || missing "Homebrew (brew)" ;;
nix) use_nix || missing "Nix" ;;
'')
    use_pacman || use_apt || use_dnf || use_zypper || use_brew || use_nix || {
        if [ "$(uname -s)" = Darwin ]; then missing "Homebrew (brew)"
        else missing "a package manager (pacman, apt, dnf, zypper or nix)"; fi
    }
    ;;
*) fail 10 "unknown packageManager '$packageManager' (pacman, apt, dnf, zypper, brew, nix)" ;;
esac

furnish_main "$@"
