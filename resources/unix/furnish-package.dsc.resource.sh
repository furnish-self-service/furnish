#!/bin/sh
# Furnish/Package: manage a package with the system package manager.
# DSC passes the resource properties as env vars: $name and (optionally) $_exist.
#
#   get                  print the current state
#   set [--what-if]      converge to the desired state; with --what-if only describe the
#                        steps (in _metadata.whatIf) and print the state it would lead to
set -eu

# Own exit codes start at 10, so package-manager failures (usually 1) stay unmapped
# and DSC shows their stderr instead of a canned description.
[ -n "${name:-}" ] || { echo '{"error":"property name is required"}' >&2; exit 10; }

# Run privileged commands: directly as root; from the GUI (FURNISH_GRAPHICAL) always via the
# graphical pkexec dialog, even if a terminal exists; otherwise via sudo, or pkexec without a terminal.
as_root() {
    if [ "$(id -u)" = 0 ]; then "$@"
    elif [ -n "${FURNISH_GRAPHICAL:-}" ]; then pkexec_or_cancel "$@"
    elif sudo -n true 2>/dev/null; then sudo -n "$@"
    elif (: </dev/tty) 2>/dev/null; then sudo "$@" </dev/tty
    else pkexec_or_cancel "$@"
    fi
}

# pkexec exits 126 (dialog dismissed) or 127 (not authorized) without running anything:
# that is the user saying no, not a failure. Exit 12 tells furnish the step was cancelled.
pkexec_or_cancel() {
    rc=0
    pkexec "$@" || rc=$?
    if [ "$rc" = 126 ] || [ "$rc" = 127 ]; then
        echo '{"warn":"administrator rights were not granted"}' >&2
        exit 12
    fi
    return "$rc"
}

# Runs a command so furnish can force-stop it: when the file $FURNISH_STOP_FILE appears, the
# command gets SIGTERM and the step exits 13. The watcher runs with the command (as root if
# needed), because furnish itself may not signal root processes. SIGTERM, not SIGINT:
# non-interactive shells start background jobs with SIGINT ignored.
WATCH='stop=$1; shift; "$@" & pid=$!
while kill -0 "$pid" 2>/dev/null; do
    if [ -n "$stop" ] && [ -e "$stop" ]; then kill -TERM "$pid"; wait "$pid"; exit 13; fi
    sleep 0.3
done
wait "$pid"'
stoppable() { sh -c "$WATCH" furnish-step "${FURNISH_STOP_FILE:-}" "$@"; }

# rpm -q prints "package X is not installed" on stdout, so only print on success.
rpm_version() { v=$(rpm -q --qf '%{VERSION}-%{RELEASE}' "$name" 2>/dev/null) && echo "$v"; }

# Each backend defines: manager, admin (needs root?), install_cmd, remove_cmd and
# installed_version (prints the version or nothing). Commands are plain words: $name is
# restricted to [A-Za-z0-9@._+-] by the schema, so word splitting is safe.
admin=true
if command -v pacman >/dev/null 2>&1; then
    manager=pacman
    installed_version() { pacman -Q "$name" 2>/dev/null | cut -d' ' -f2; }
    install_cmd="pacman -S --noconfirm --needed $name"
    remove_cmd="pacman -R --noconfirm $name"
elif command -v apt-get >/dev/null 2>&1; then
    manager=apt
    installed_version() { dpkg-query -W -f='${db:Status-Status} ${Version}' "$name" 2>/dev/null | sed -n 's/^installed //p'; }
    install_cmd="env DEBIAN_FRONTEND=noninteractive apt-get install -y $name"
    remove_cmd="env DEBIAN_FRONTEND=noninteractive apt-get remove -y $name"
elif command -v dnf >/dev/null 2>&1; then
    manager=dnf
    installed_version() { rpm_version; }
    install_cmd="dnf install -y $name"
    remove_cmd="dnf remove -y $name"
elif command -v zypper >/dev/null 2>&1; then
    manager=zypper
    installed_version() { rpm_version; }
    install_cmd="zypper --non-interactive install $name"
    remove_cmd="zypper --non-interactive remove $name"
elif command -v brew >/dev/null 2>&1; then
    manager=Homebrew
    admin=false # Homebrew refuses to run as root
    installed_version() { brew list --versions "$name" 2>/dev/null | cut -d' ' -f2; }
    install_cmd="brew install $name"
    remove_cmd="brew uninstall $name"
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
    install_cmd="nix profile install nixpkgs#$name"
    remove_cmd="nix profile remove $name"
else
    echo '{"error":"no supported package manager found (pacman, apt-get, dnf, zypper, brew, nix)"}' >&2
    exit 11
fi

# Prints the state object without its closing brace. $1: version, "?" (installed, version
# not known yet: what-if of an install) or empty (not installed).
state() {
    case "$1" in
    '') printf '{"name":"%s","_exist":false' "$name" ;;
    '?') printf '{"name":"%s","_exist":true' "$name" ;;
    *) printf '{"name":"%s","_exist":true,"version":"%s"' "$name" "$1" ;;
    esac
}

case "${1:-}" in
get)
    state "$(installed_version || true)"
    echo '}'
    ;;
set)
    version=$(installed_version || true)
    want=${_exist:-true}
    cmd=
    if [ "$want" = true ] && [ -z "$version" ]; then
        cmd=$install_cmd step="Install $name with $manager" after=?
    elif [ "$want" != true ] && [ -n "$version" ]; then
        cmd=$remove_cmd step="Remove $name $version with $manager" after=
    elif [ -n "$version" ]; then
        step="$name $version is already installed" after=$version
    else
        step="$name is not installed" after=
    fi

    if [ "${2:-}" = --what-if ]; then
        # Describe, don't act. The whatIf messages are DSC's convention for what-if details.
        detail=
        if [ -n "$cmd" ]; then
            how="as your user"; $admin && how="as administrator"
            detail=",\"Run $how: $cmd\""
        fi
        state "$after"
        printf ',"_metadata":{"whatIf":["%s"%s]}}\n' "$step" "$detail"
        exit 0
    fi

    if [ -n "$cmd" ]; then
        how="as your user"; $admin && how="as administrator"
        echo "{\"info\":\"$step: running $how: $cmd\"}" >&2
        # Package manager output goes to stderr: DSC reads stdout as JSON.
        if $admin; then as_root sh -c "$WATCH" furnish-step "${FURNISH_STOP_FILE:-}" $cmd >&2
        else stoppable $cmd >&2; fi
    fi
    state "$(installed_version || true)"
    echo '}'
    ;;
*)
    echo '{"error":"usage: get|set [--what-if]"}' >&2
    exit 10
    ;;
esac
