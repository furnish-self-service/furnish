# Homebrew for root on macOS (the furnish service). Homebrew refuses to run as root, so root runs
# brew as the account that owns the Homebrew installation, without a password: root may become
# any user. Sourced by furnish-package.dsc.resource.sh. `furnish service install` runs it as
# `sh -c <this file> furnish-homebrew-check` and gets the owner's name or why there is none.
#
# Why this is safe: brew runs code from its installation (Homebrew itself, taps, formulae, casks'
# scripts) as the owner, and `furnish service install --homebrew-sudo` lets the owner run sudo
# without a password, because casks that run installer packages call sudo themselves. Whoever can
# run code as the owner can therefore become root. brew_owner refuses unless that is nobody but
# administrators and root:
#   - the owner is a member of the admin group (an administrator can become root anyway), and
#   - nobody else can change the installation: the folders above it belong to root or the owner,
#     and the prefix and repository, their first two levels and all of Homebrew's own code
#     (bin, Library/Homebrew, Library/Taps) belong to the owner or root, aren't writable by
#     others, and are group-writable only by admin or wheel. The folders above and the prefix
#     itself must not have an ACL that lets anyone write.
# If someone without administrator rights owns the installation (they installed Homebrew
# themselves), the service doesn't use Homebrew: an administrator has to give the installation to
# an administrator account (see docs/site/admins.md).

# Sets brew (the program), brew_user, brew_home, brew_prefix and brew_repo. Returns 1 if there is
# no Homebrew, 2 with brew_problem set if it isn't safe to use.
brew_owner() {
    brew_user= brew_problem=
    # The two standard places, not PATH: /usr/local/bin/brew is only a link.
    if [ -x /opt/homebrew/bin/brew ]; then brew_prefix=/opt/homebrew brew_repo=/opt/homebrew
    elif [ -x /usr/local/Homebrew/bin/brew ]; then brew_prefix=/usr/local brew_repo=/usr/local/Homebrew
    else return 1; fi
    brew=$brew_repo/bin/brew
    owner=$(/usr/bin/stat -f %Su "$brew_repo") || { brew_problem="can't tell who owns $brew_repo"; return 2; }
    case $owner in
    root) brew_problem="$brew_repo belongs to root, and Homebrew doesn't run as root"; return 2 ;;
    '' | -* | *[!A-Za-z0-9_.-]*) brew_problem="$brew_repo belongs to '$owner', a name furnish doesn't accept"; return 2 ;;
    esac
    if ! /usr/sbin/dseditgroup -o checkmember -m "$owner" admin >/dev/null 2>&1; then
        brew_problem="$brew_repo belongs to $owner, who isn't an administrator"
        return 2
    fi
    # The folders above, and the prefix and repository themselves.
    for d in $(p=$brew_repo; while :; do echo "$p"; [ "$p" = / ] && break; p=$(dirname "$p"); done) $brew_prefix; do
        set -- $(/usr/bin/stat -f '%Su %Sg %Lp' "$d")
        if [ "$1" != root ] && [ "$1" != "$owner" ]; then brew_problem="$d belongs to $1"; return 2; fi
        mode=$3
        case $mode in *[2367]) brew_problem="everyone can write to $d"; return 2 ;; esac
        case $mode in *[2367]?)
            [ "$2" = admin ] || [ "$2" = wheel ] || { brew_problem="the group $2 can write to $d"; return 2; } ;;
        esac
        if /bin/ls -led "$d" | tail -n +2 | grep -E ' allow ' | grep -qE 'write|append|add_|delete_child|chown|security'; then
            brew_problem="an access control entry lets others write to $d"
            return 2
        fi
    done
    bad=$( {
        /usr/bin/find "$brew_prefix" "$brew_repo" -maxdepth 2 ! -type l \( -perm -0002 -o \( -perm -0020 ! -group admin ! -group wheel \) -o \( ! -user "$owner" ! -user root \) \) -print
        /usr/bin/find "$brew_repo/bin" "$brew_repo/Library/Homebrew" "$brew_repo/Library/Taps" ! -type l \( -perm -0002 -o \( -perm -0020 ! -group admin ! -group wheel \) -o \( ! -user "$owner" ! -user root \) \) -print
    } 2>/dev/null | head -n 1)
    if [ -n "$bad" ]; then
        brew_problem="$bad belongs to someone besides $owner and root, or others can write to it"
        return 2
    fi
    brew_home=$(/usr/bin/dscl . -read "/Users/$owner" NFSHomeDirectory 2>/dev/null | sed -n 's/^NFSHomeDirectory: //p')
    [ -d "$brew_home" ] || { brew_problem="$owner has no home folder"; return 2; }
    brew_user=$owner
}

# `run_brew <runner> <brew arguments>`: runs `<runner> brew <arguments>` (runner: `command` or
# `run_step`). As brew_user when set, then in a clean environment: nothing of root's or DSC's
# (the resource's properties) reaches brew. Passes HOMEBREW_NO_AUTO_UPDATE on.
run_brew() {
    runner=$1
    shift
    if [ -n "${brew_user:-}" ]; then
        set -- /usr/bin/sudo -n -u "$brew_user" -- /usr/bin/env -i "HOME=$brew_home" "USER=$brew_user" \
            "LOGNAME=$brew_user" "PATH=$brew_prefix/bin:/usr/bin:/bin:/usr/sbin:/sbin" LANG=en_US.UTF-8 \
            "HOMEBREW_NO_AUTO_UPDATE=${HOMEBREW_NO_AUTO_UPDATE:-}" HOMEBREW_NO_ANALYTICS=1 HOMEBREW_NO_ENV_HINTS=1 \
            "$brew" "$@"
    elif [ -n "${HOMEBREW_NO_AUTO_UPDATE:-}" ]; then
        set -- env "HOMEBREW_NO_AUTO_UPDATE=$HOMEBREW_NO_AUTO_UPDATE" brew "$@"
    else
        set -- brew "$@"
    fi
    $runner "$@"
}

if [ "${0##*/}" = furnish-homebrew-check ]; then
    rc=0
    brew_owner || rc=$?
    case $rc in
    0) echo "$brew_user $brew_repo" ;;
    1) echo "no Homebrew in /opt/homebrew or /usr/local/Homebrew" >&2 ;;
    *) echo "$brew_problem" >&2 ;;
    esac
    exit "$rc"
fi
