#!/bin/sh
# Furnish/Package: manage a package with the system package manager.
# DSC passes the resource properties as env vars: $name and (optionally) $_exist.
set -eu

: "${name:?property 'name' is required}"

# Run privileged commands via sudo when interactive, pkexec otherwise (GUI sessions).
as_root() {
    if [ "$(id -u)" = 0 ]; then "$@"
    elif (: </dev/tty) 2>/dev/null; then sudo "$@" </dev/tty
    else pkexec "$@"
    fi
}

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
    installed_version() { rpm -q --qf '%{VERSION}-%{RELEASE}' "$name" 2>/dev/null || true; }
    install() { as_root dnf install -y "$name"; }
    remove() { as_root dnf remove -y "$name"; }
elif command -v zypper >/dev/null 2>&1; then
    installed_version() { rpm -q --qf '%{VERSION}-%{RELEASE}' "$name" 2>/dev/null || true; }
    install() { as_root zypper --non-interactive install "$name"; }
    remove() { as_root zypper --non-interactive remove "$name"; }
elif command -v brew >/dev/null 2>&1; then
    # Homebrew refuses to run as root, so no as_root here.
    installed_version() { brew list --versions "$name" 2>/dev/null | cut -d' ' -f2; }
    install() { brew install "$name"; }
    remove() { brew uninstall "$name"; }
else
    echo "Furnish/Package: no supported package manager found (pacman, apt-get, dnf, zypper, brew)" >&2
    exit 2
fi

case "${1:-}" in
get)
    version=$(installed_version)
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
    echo "usage: $0 get|set" >&2
    exit 1
    ;;
esac
