#!/bin/sh
# Furnish/Package: manage a package with the system package manager.
# DSC passes the resource properties as env vars: $name and (optionally) $_exist.
set -eu

# Own exit codes start at 10, so package-manager failures (usually 1) stay unmapped
# and DSC shows their stderr instead of a canned description.
[ -n "${name:-}" ] || { echo '{"error":"property name is required"}' >&2; exit 10; }

# Run privileged commands: directly as root, via passwordless or interactive sudo,
# or via pkexec when there is no terminal (GUI sessions).
as_root() {
    if [ "$(id -u)" = 0 ]; then "$@"
    elif sudo -n true 2>/dev/null; then sudo -n "$@"
    elif (: </dev/tty) 2>/dev/null; then sudo "$@" </dev/tty
    else pkexec "$@"
    fi
}

# rpm -q prints "package X is not installed" on stdout, so only print on success.
rpm_version() { v=$(rpm -q --qf '%{VERSION}-%{RELEASE}' "$name" 2>/dev/null) && echo "$v"; }

# Each backend defines: installed_version (prints version or nothing), install, remove.
if command -v pacman >/dev/null 2>&1; then
    installed_version() { pacman -Q "$name" 2>/dev/null | cut -d' ' -f2; }
    install() { as_root pacman -S --noconfirm --needed "$name"; }
    remove() { as_root pacman -R --noconfirm "$name"; }
elif command -v apt-get >/dev/null 2>&1; then
    installed_version() { dpkg-query -W -f='${db:Status-Status} ${Version}' "$name" 2>/dev/null | sed -n 's/^installed //p'; }
    install() { as_root env DEBIAN_FRONTEND=noninteractive apt-get install -y "$name"; }
    remove() { as_root env DEBIAN_FRONTEND=noninteractive apt-get remove -y "$name"; }
elif command -v dnf >/dev/null 2>&1; then
    installed_version() { rpm_version; }
    install() { as_root dnf install -y "$name"; }
    remove() { as_root dnf remove -y "$name"; }
elif command -v zypper >/dev/null 2>&1; then
    installed_version() { rpm_version; }
    install() { as_root zypper --non-interactive install "$name"; }
    remove() { as_root zypper --non-interactive remove "$name"; }
elif command -v brew >/dev/null 2>&1; then
    # Homebrew refuses to run as root, so no as_root here.
    installed_version() { brew list --versions "$name" 2>/dev/null | cut -d' ' -f2; }
    install() { brew install "$name"; }
    remove() { brew uninstall "$name"; }
elif command -v nix >/dev/null 2>&1; then
    # NixOS (or nix-only systems): per-user profile from nixpkgs, needs no root.
    nix() { command nix --extra-experimental-features 'nix-command flakes' "$@"; }
    installed_version() {
        nix profile list --json 2>/dev/null | tr '{,' '\n\n' | grep -q "^\"$name\":" || return 0
        path=$(nix profile list --json | grep -o "/nix/store/[a-z0-9]*-$name-[^\"]*" | head -n1)
        echo "${path##*-"$name"-}"
    }
    install() { nix profile install "nixpkgs#$name"; }
    remove() { nix profile remove "$name"; }
else
    echo '{"error":"no supported package manager found (pacman, apt-get, dnf, zypper, brew, nix)"}' >&2
    exit 11
fi

case "${1:-}" in
get)
    version=$(installed_version || true)
    if [ -n "$version" ]; then
        printf '{"name":"%s","_exist":true,"version":"%s"}\n' "$name" "$version"
    else
        printf '{"name":"%s","_exist":false}\n' "$name"
    fi
    ;;
set)
    # Package manager output goes to stderr: DSC reads stdout as JSON.
    if [ "${_exist:-true}" = true ]; then install >&2; else remove >&2; fi
    ;;
*)
    echo '{"error":"usage: get|set"}' >&2
    exit 10
    ;;
esac
