# Shared by furnish's bundled resources; sourced, not run. A resource defines:
#
#   identity            JSON members naming the instance, e.g. "$(json_member name "$name")"
#   subject             what the steps talk about, e.g. "htop"
#   via                 what installs it, e.g. "pacman"
#   admin               true if installing/removing needs administrator rights
#   run_as              optional: the account the commands run as, if not the caller's
#   install_desc, remove_desc   the commands as shown in previews
#   do_install, do_remove       functions that run them through run_step
#   installed_version   function printing the installed version, "?" if unknown, or nothing
#   newer_version       optional function printing the version an update would bring, or nothing
#   do_upgrade, upgrade_desc    optional: how to update (default: install again)
#   prepare             optional function run once before anything else (e.g. a slow lookup
#                       whose results the other functions read)
#   extra_state         optional function printing more state members (",\"x\":..."); $1 is the
#                       installed version as for `state`
#   cant_remove         optional: why removing isn't possible here; set then refuses with exit 14
#   scheme              how its versions compare (versionScheme): pacman, dpkg, rpm, brew, nix,
#                       winget or generic
#   latest_version      optional function printing the newest version on offer, installed or not,
#                       or nothing if unknown (only called with reportVersions or targetVersion)
#   available_versions  optional function printing each version it can install, one per line
#                       (default: latest_version; only called with reportVersions=all)
#   pin                 true if it can install a given version: then do_install_version and
#                       version_desc (functions) install exactly $targetVersion and describe that
#                       command. Else only the version on offer (latest_version) can be the target
#
# and then calls `furnish_main "$@"`:
#
#   get                 print the current state
#   test                print the current state with _inDesiredState: installed means
#                       up to date, so an available update counts as not in desired state
#   set [--what-if]     converge to the desired state (install, update or remove); with
#                       --what-if only describe the steps (in _metadata.whatIf)
#
# Two write-only properties come from furnish's required versions (furnish compares, the resource
# reports and obeys):
#
#   reportVersions      latest or all: every state also has versionScheme, via, latestVersion (if
#                       known) and, with all, availableVersions
#   targetVersion       install or update to exactly this version; test is in desired state iff
#                       the installed version is this one
#
# Exit codes: 10 invalid input, 11 a needed tool is missing (see `missing`), 12 administrator rights
# not granted, 13 stopped by the user, 14 removing isn't possible here (see `cant_remove`). Tool
# failures keep their own code so DSC shows their stderr.

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
    extra_state "$1"
    version_state
}

# The versions on offer, once per run (the lookups can be slow, e.g. nix eval): sets offered and
# offered_all (one per line, without duplicates).
versions_loaded=false offered= offered_all=
load_versions() {
    $versions_loaded && return 0
    versions_loaded=true
    offered=$(latest_version || true)
    [ "$reportVersions" = all ] || return 0
    offered_all=
    for _v in $(available_versions || true); do
        case "
$offered_all
" in *"
$_v
"*) ;; *) offered_all="${offered_all:+$offered_all
}$_v" ;; esac
    done
}
# The members reportVersions asks for.
version_state() {
    [ -n "$reportVersions" ] || return 0
    load_versions
    printf ',"versionScheme":%s,"via":%s' "$(json "$scheme")" "$(json "$via")"
    [ -z "$offered" ] || printf ',"latestVersion":%s' "$(json "$offered")"
    [ "$reportVersions" = all ] || return 0
    _list= _sep=
    for _v in $offered_all; do _list="$_list$_sep$(json "$_v")" _sep=,; done
    printf ',"availableVersions":[%s]' "$_list"
}

prepare() { :; }
check_external_install() { :; }
extra_state() { :; }
scheme=generic
latest_version() { :; }
available_versions() { latest_version; }
pin=false
cant_remove=
newer_version() { :; }
upgrade_desc=
do_upgrade() { do_install; }

reportVersions=${reportVersions:-} targetVersion=${targetVersion:-}

# set with targetVersion (and _exist true): sets action, step, desc and after like furnish_main.
target_step() {
    t=$targetVersion
    if [ "$version" = "$t" ]; then
        step="$subject $t is installed, the required version" after=$t
        return
    fi
    if $pin; then
        action=install_version desc=$(version_desc)
    else
        # Only the version on offer: a plain install or update brings it.
        load_versions
        [ "$offered" = "$t" ] || fail 10 "$via can't install a specific version ($t)"
        if [ -z "$version" ]; then action=install desc=$install_desc
        else action=upgrade desc=${upgrade_desc:-$install_desc}; fi
    fi
    if [ -z "$version" ]; then step="Install $subject $t with $via"
    elif [ "$version" = '?' ]; then step="Update $subject to $t with $via"
    else step="Update $subject from $version to $t with $via"; fi
    after=$t
}

furnish_main() {
    case $reportVersions in '' | latest | all) ;; *) fail 10 "reportVersions must be latest or all: $reportVersions" ;; esac
    # Not an option to the package manager (the schema says so too; this holds without it).
    case $targetVersion in
    -*) fail 10 "targetVersion starts with '-': $targetVersion" ;;
    *[!A-Za-z0-9.+~:_^-]*) fail 10 "targetVersion has characters a version can't have: $targetVersion" ;;
    esac
    prepare
    version=$(installed_version || true)
    check_external_install
    case "${1:-}" in
    get)
        state "$version"
        echo '}'
        ;;
    test)
        if [ "${_exist:-true}" = true ] && [ -n "$targetVersion" ]; then
            ok=false
            [ "$version" = "$targetVersion" ] && ok=true
        elif [ "${_exist:-true}" = true ]; then
            ok=false
            [ -n "$version" ] && [ -z "$(newer_version || true)" ] && ok=true
        else
            ok=true
            [ -n "$version" ] && ok=false
        fi
        state "$version"
        printf ',"_inDesiredState":%s}\n' "$ok"
        ;;
    set)
        newer=
        [ -n "$version" ] && [ "${_exist:-true}" = true ] && [ -z "$targetVersion" ] && newer=$(newer_version || true)
        shown=$subject; [ -n "$version" ] && [ "$version" != '?' ] && shown="$subject $version"
        action=
        if [ "${_exist:-true}" = true ] && [ -n "$targetVersion" ]; then
            target_step
        elif [ "${_exist:-true}" = true ] && [ -z "$version" ]; then
            action=install step="Install $subject with $via" desc=$install_desc after='?'
        elif [ -n "$newer" ]; then
            action=upgrade step="Update $subject from $version to $newer with $via" desc=${upgrade_desc:-$install_desc} after=$newer
        elif [ "${_exist:-true}" != true ] && [ -n "$version" ] && [ -n "$cant_remove" ]; then
            # Plain text and an unmapped exit code, so DSC keeps the line in its error.
            [ "${2:-}" = --what-if ] || { echo "furnish: $cant_remove" >&2; exit 14; }
            step=$cant_remove after=$version
        elif [ "${_exist:-true}" != true ] && [ -n "$version" ]; then
            action=remove step="Remove $shown with $via" desc=$remove_desc after=
        elif [ -n "$version" ]; then
            step="$shown is installed and up to date" after=$version
        else
            step="$subject is not installed" after=
        fi
        how="as your user"; $admin && how="as administrator"; [ -n "${run_as:-}" ] && how="as $run_as"

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
        fail 10 "usage: get|test|set [--what-if]"
        ;;
    esac
}
