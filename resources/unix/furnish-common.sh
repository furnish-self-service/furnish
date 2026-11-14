# Shared by furnish's bundled resources; sourced, not run. A resource defines:
#
#   identity            JSON members naming the instance, e.g. "$(json_member name "$name")"
#   subject             what the steps talk about, e.g. "htop"
#   via                 what installs it, e.g. "pacman"
#   admin               true if installing/removing needs administrator rights
#   install_desc, remove_desc   the commands as shown in previews
#   do_install, do_remove       functions that run them through run_step
#   installed_version   function printing the installed version, "?" if unknown, or nothing
#
# and then calls `furnish_main "$@"`:
#
#   get                 print the current state
#   set [--what-if]     converge to the desired state; with --what-if only describe the
#                       steps (in _metadata.whatIf) and print the state it would lead to
#
# Exit codes: 10 invalid input, 11 a needed tool is missing (see `missing`), 12 administrator rights
# not granted, 13 stopped by the user. Tool failures keep their own code so DSC shows their stderr.

# JSON string: escapes backslashes and quotes. Pure sh: some systems (nixos/nix) have no sed.
json() {
    s=$1 out=
    while :; do
        case "$s" in
        *[\\\"]*)
            pre=${s%%[\\\"]*} rest=${s#"$pre"}
            out="$out$pre\\${rest%"${rest#?}"}" s=${rest#?}
            ;;
        *) out="$out$s"; break ;;
        esac
    done
    printf '"%s"' "$out"
}
json_member() { printf '%s:%s' "$(json "$1")" "$(json "$2")"; }
fail() { echo "{\"error\":$(json "$2")}" >&2; exit "$1"; }
# A tool this resource needs is not installed. Plain text (not JSON) and an unmapped exit code,
# so DSC keeps the line in its error: furnish shows "Needs <what>" instead of a failure.
missing() { echo "furnish: needs $1" >&2; exit 11; }

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

# Run one install/remove command (arguments as given, no word splitting), with admin rights if
# the resource needs them. Its output goes to stderr: DSC reads stdout as JSON.
run_step() {
    if $admin; then as_root sh -c "$WATCH" furnish-step "${FURNISH_STOP_FILE:-}" "$@" >&2
    else sh -c "$WATCH" furnish-step "${FURNISH_STOP_FILE:-}" "$@" >&2; fi
}

# Prints the state object without its closing brace. $1: version, "?" (installed, version
# unknown) or empty (not installed).
state() {
    case "$1" in
    '') printf '{%s,"_exist":false' "$identity" ;;
    '?') printf '{%s,"_exist":true' "$identity" ;;
    *) printf '{%s,"_exist":true,"version":%s' "$identity" "$(json "$1")" ;;
    esac
}

furnish_main() {
    case "${1:-}" in
    get)
        state "$(installed_version || true)"
        echo '}'
        ;;
    set)
        version=$(installed_version || true)
        shown=$subject; [ -n "$version" ] && [ "$version" != '?' ] && shown="$subject $version"
        action=
        if [ "${_exist:-true}" = true ] && [ -z "$version" ]; then
            action=install step="Install $subject with $via" desc=$install_desc after='?'
        elif [ "${_exist:-true}" != true ] && [ -n "$version" ]; then
            action=remove step="Remove $shown with $via" desc=$remove_desc after=
        elif [ -n "$version" ]; then
            step="$shown is already installed" after=$version
        else
            step="$subject is not installed" after=
        fi
        how="as your user"; $admin && how="as administrator"

        if [ "${2:-}" = --what-if ]; then
            # Describe, don't act. The whatIf messages are DSC's convention for what-if details.
            detail=
            [ -n "$action" ] && detail=",$(json "Run $how: $desc")"
            state "$after"
            printf ',"_metadata":{"whatIf":[%s%s]}}\n' "$(json "$step")" "$detail"
            exit 0
        fi

        if [ -n "$action" ]; then
            echo "{\"info\":$(json "$step: running $how: $desc")}" >&2
            "do_$action"
        fi
        state "$(installed_version || true)"
        echo '}'
        ;;
    *)
        fail 10 "usage: get|set [--what-if]"
        ;;
    esac
}
