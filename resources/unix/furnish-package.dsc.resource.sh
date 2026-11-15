#!/bin/sh
# Furnish/Package: manage a package with the system package manager.
# DSC passes the resource properties as env vars: $name, $packageManager (optional: pacman, apt,
# dnf, zypper, brew, nix; default: the first one found), $<manager>Name (optional: the package's
# name for that package manager, e.g. nixName), $nixpkgs (optional flake reference) and $_exist.
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
    # Local sync database only ("name 1.0-1 -> 1.1-1").
    newer_version() { pacman -Qu "$name" 2>/dev/null | cut -d' ' -f4; }
}
use_apt() {
    have apt-get || return 1
    via=apt admin=true
    installed_version() { dpkg-query -W -f='${db:Status-Status} ${Version}' "$name" 2>/dev/null | sed -n 's/^installed //p'; }
    do_install() { run_step env DEBIAN_FRONTEND=noninteractive apt-get install -y "$name"; }
    do_remove() { run_step env DEBIAN_FRONTEND=noninteractive apt-get remove -y "$name"; }
    install_desc="apt-get install -y $name" remove_desc="apt-get remove -y $name"
    # Local package lists only ("name/suite 1.1 amd64 [upgradable from: 1.0]").
    newer_version() { apt list --upgradable 2>/dev/null | grep "^$name/" | cut -d' ' -f2; }
    do_upgrade() { run_step env DEBIAN_FRONTEND=noninteractive apt-get install -y --only-upgrade "$name"; }
    upgrade_desc="apt-get install -y --only-upgrade $name"
}
use_dnf() {
    have dnf || return 1
    via=dnf admin=true
    installed_version() { rpm_version; }
    do_install() { run_step dnf install -y "$name"; }
    do_remove() { run_step dnf remove -y "$name"; }
    install_desc="dnf install -y $name" remove_desc="dnf remove -y $name"
    # Cached metadata only (-C): no download during a status check.
    newer_version() { dnf -C -q check-update "$name" 2>/dev/null | grep "^$name\\." | tr -s ' ' | cut -d' ' -f2; }
    do_upgrade() { run_step dnf upgrade -y "$name"; }
    upgrade_desc="dnf upgrade -y $name"
}
use_zypper() {
    have zypper || return 1
    via=zypper admin=true
    installed_version() { rpm_version; }
    do_install() { run_step zypper --non-interactive install "$name"; }
    do_remove() { run_step zypper --non-interactive remove "$name"; }
    install_desc="zypper --non-interactive install $name" remove_desc="zypper --non-interactive remove $name"
    # Without refreshing repositories: "v | repo | name | current | available | arch".
    newer_version() {
        line=$(zypper --no-refresh -q list-updates 2>/dev/null | grep "| $name " | head -n1) || return 0
        [ -n "$line" ] && echo "$line" | cut -d'|' -f5 | tr -d ' '
    }
    do_upgrade() { run_step zypper --non-interactive update "$name"; }
    upgrade_desc="zypper --non-interactive update $name"
}
use_brew() {
    have brew || return 1
    via=Homebrew admin=false # Homebrew refuses to run as root
    installed_version() { brew list --versions "$name" 2>/dev/null | cut -d' ' -f2; }
    do_install() { run_step brew install "$name"; }
    do_remove() { run_step brew uninstall "$name"; }
    install_desc="brew install $name" remove_desc="brew uninstall $name"
    # "name (1.0) < 1.1"; no auto-update of Homebrew itself during a status check.
    # brew outdated exits non-zero exactly when something is outdated.
    newer_version() { line=$(HOMEBREW_NO_AUTO_UPDATE=1 brew outdated --verbose "$name" 2>/dev/null || true); [ -n "$line" ] && echo "${line##* }"; }
    do_upgrade() { run_step brew upgrade "$name"; }
    upgrade_desc="brew upgrade $name"
}
use_nix() {
    # Nix profiles, no root needed: the user's own profile. As root (the furnish service), the
    # shared profile /nix/var/nix/profiles/furnish, which the NixOS module puts on everyone's PATH.
    have nix || return 1
    via=nix admin=false
    # Add to the user's settings rather than replacing them.
    export NIX_CONFIG="${NIX_CONFIG:+$NIX_CONFIG
}extra-experimental-features = nix-command flakes"
    shared=/nix/var/nix/profiles/furnish
    ref=${nixpkgs:-nixpkgs}
    if [ "$(id -u)" = 0 ]; then profile="--profile $shared"; else profile=""; fi
    # A profile names an entry after the package's last attribute: kdePackages.kdenlive → kdenlive.
    entry=${name##*.}
    # The version in a profile ("" if it doesn't hold the package).
    version_in() {
        nix profile list $1 --json 2>/dev/null | tr '{,' '\n\n' | grep -q "^\"$entry\":" || return 0
        path=$(nix profile list $1 --json | grep -o "/nix/store/[a-z0-9]*-$entry-[^\"]*" | head -n1)
        echo "${path##*-"$entry"-}"
    }
    # Installed for this user or for everyone.
    installed_version() {
        v=$(version_in "")
        [ -n "$v" ] || { [ -e "$shared" ] && v=$(version_in "--profile $shared"); }
        echo "$v"
    }
    do_install() { run_step nix profile install $profile "$ref#$name"; }
    do_remove() { run_step nix profile remove $profile "$entry"; }
    # Not `nix profile upgrade`: it re-resolves the reference installed from, not the current one.
    do_upgrade() { do_remove && do_install; }
    install_desc="nix profile install $profile $ref#$name" remove_desc="nix profile remove $profile $entry"
    upgrade_desc="nix profile remove $profile $entry && nix profile install $profile $ref#$name"
    # The version in the referenced nixpkgs, if it differs from the installed one.
    newer_version() {
        # Offline, say, the check fails: the app counts as up to date, and the activity log says why.
        # Errors go to a private file (mktemp: random name, only ours), warnings on success are dropped.
        err=$(mktemp) || return 0
        if ! available=$(nix eval --raw "$ref#$name.version" 2>"$err"); then
            echo "{\"warn\":$(json "couldn't check $ref for a newer $name: $(tail -n1 "$err")")}" >&2
            rm -f "$err"
            return 0
        fi
        rm -f "$err"
        [ -n "$available" ] && [ "$available" != "$(installed_version)" ] && echo "$available"
    }
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

# A package can have another name per package manager (e.g. code / vscode).
eval "override=\${${via}Name:-}"
if [ -n "$override" ]; then name=$override subject=$override; fi

furnish_main "$@"
