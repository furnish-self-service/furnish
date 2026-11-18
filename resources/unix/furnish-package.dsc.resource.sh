#!/bin/sh
# Furnish/Package: manage a package with the system package manager.
# The native resource passes its resolved package name and manager, plus optional
# nixpkgs, command, _exist, reportVersions and targetVersion (see furnish-common.sh).
set -eu
. "$(dirname "$0")/furnish-common.sh"

[ -n "${name:-}" ] || fail 10 "property name is required"
identity=$(json_member name "$name")
[ -n "${packageManager:-}" ] && identity="$identity,$(json_member packageManager "$packageManager")"
subject=$name

# rpm -q prints "package X is not installed" on stdout, so only print on success. The epoch only
# when it isn't 0, as dnf and zypper show versions.
rpm_version() { v=$(rpm -q --qf '%|EPOCH?{%{EPOCH}:}:{}|%{VERSION}-%{RELEASE}' -- "$name" 2>/dev/null) && echo "$v"; }
have() { command -v "$1" >/dev/null 2>&1; }
# The version lookups parse the tools' output: in English.
english() { LC_ALL=C "$@"; }
# Each word on stdin once, in order (some systems have no awk).
uniq_words() {
    seen=
    for w in $(cat); do
        case " $seen " in *" $w "*) ;; *) seen="$seen $w"; echo "$w" ;; esac
    done
}

# Each backend: returns 1 if its tool is missing, else sets via, admin, the commands and
# installed_version. $name is restricted to [A-Za-z0-9@._+-] by the schema and can't start with
# `-` (checked again below); `--` before it where the tool takes that.
use_pacman() {
    have pacman || return 1
    manager=pacman via=pacman admin=true
    installed_version() { pacman -Q -- "$name" 2>/dev/null | cut -d' ' -f2; }
    do_install() { run_step pacman -S --noconfirm --needed -- "$name"; }
    do_remove() { run_step pacman -R --noconfirm -- "$name"; }
    install_desc="pacman -S --noconfirm --needed $name" remove_desc="pacman -R --noconfirm $name"
    # Local sync database only ("name 1.0-1 -> 1.1-1").
    newer_version() { pacman -Qu -- "$name" 2>/dev/null | cut -d' ' -f4; }
    # pacman installs only the sync database's version.
    scheme=pacman
    latest_version() { english pacman -Si -- "$name" 2>/dev/null | sed -n 's/^Version *: //p' | head -n 1; }
}
use_apt() {
    have apt-get || return 1
    manager=apt via=apt admin=true
    installed_version() { dpkg-query -W -f='${db:Status-Status} ${Version}' -- "$name" 2>/dev/null | sed -n 's/^installed //p'; }
    do_install() { run_step env DEBIAN_FRONTEND=noninteractive apt-get install -y -- "$name"; }
    do_remove() { run_step env DEBIAN_FRONTEND=noninteractive apt-get remove -y -- "$name"; }
    install_desc="apt-get install -y $name" remove_desc="apt-get remove -y $name"
    # Local package lists only ("name/suite 1.1 amd64 [upgradable from: 1.0]").
    newer_version() { apt list --upgradable 2>/dev/null | grep "^$name/" | cut -d' ' -f2; }
    do_upgrade() { run_step env DEBIAN_FRONTEND=noninteractive apt-get install -y --only-upgrade -- "$name"; }
    upgrade_desc="apt-get install -y --only-upgrade $name"
    scheme=dpkg pin=true
    latest_version() { english apt-cache policy -- "$name" 2>/dev/null | sed -n 's/^ *Candidate: //p' | grep -v '^(none)$' | head -n 1; }
    # "name | 1.2-1 | http://... Packages", newest first.
    available_versions() { english apt-cache madison -- "$name" 2>/dev/null | cut -d'|' -f2 | tr -d ' ' | uniq_words; }
    do_install_version() { run_step env DEBIAN_FRONTEND=noninteractive apt-get install -y --allow-downgrades -- "$name=$targetVersion"; }
    version_desc() { echo "apt-get install -y --allow-downgrades $name=$targetVersion"; }
}
use_dnf() {
    have dnf || return 1
    manager=dnf via=dnf admin=true
    installed_version() { rpm_version; }
    do_install() { run_step dnf install -y -- "$name"; }
    do_remove() { run_step dnf remove -y -- "$name"; }
    install_desc="dnf install -y $name" remove_desc="dnf remove -y $name"
    # Cached metadata only (-C): no download during a status check.
    newer_version() { dnf -C -q check-update -- "$name" 2>/dev/null | grep "^$name\\." | tr -s ' ' | cut -d' ' -f2; }
    do_upgrade() { run_step dnf upgrade -y -- "$name"; }
    upgrade_desc="dnf upgrade -y $name"
    # dnf install takes "name-[epoch:]version-release", also to go to an older one.
    scheme=rpm pin=true
    do_install_version() { run_step dnf install -y -- "$name-$targetVersion"; }
    version_desc() { echo "dnf install -y $name-$targetVersion"; }
    latest_version() { dnf_versions -C --latest-limit 1 | tail -n 1; }
    # From the cache; once more with fresh metadata if the cache has none.
    available_versions() { v=$(dnf_versions -C); [ -n "$v" ] || v=$(dnf_versions); echo "$v"; }
}
# Every version of $name in the repositories, oldest first, for this machine's
# architecture: "[epoch:]version-release" per line. dnf4 ends each line itself, dnf5 doesn't.
dnf_versions() {
    english dnf -q repoquery "$@" --arch "$(uname -m),noarch" --qf '%{epoch}:%{version}-%{release}\n' -- "$name" 2>/dev/null |
        while IFS= read -r l; do [ -n "$l" ] && echo "${l#0:}"; done | uniq_words
}
use_zypper() {
    have zypper || return 1
    manager=zypper via=zypper admin=true
    installed_version() { rpm_version; }
    do_install() { run_step zypper --non-interactive install -- "$name"; }
    do_remove() { run_step zypper --non-interactive remove -- "$name"; }
    install_desc="zypper --non-interactive install $name" remove_desc="zypper --non-interactive remove $name"
    # Without refreshing repositories: "v | repo | name | current | available | arch".
    newer_version() {
        line=$(zypper --no-refresh -q list-updates 2>/dev/null | grep "| $name " | head -n1) || return 0
        [ -n "$line" ] && echo "$line" | cut -d'|' -f5 | tr -d ' '
    }
    do_upgrade() { run_step zypper --non-interactive update -- "$name"; }
    upgrade_desc="zypper --non-interactive update $name"
    scheme=rpm pin=true
    do_install_version() { run_step zypper --non-interactive install --oldpackage -- "$name=$targetVersion"; }
    version_desc() { echo "zypper --non-interactive install --oldpackage $name=$targetVersion"; }
    # The version zypper would install ("Version : 1.2-1.1"; for an installed package the newest).
    latest_version() { english zypper --no-refresh -q info -- "$name" 2>/dev/null | sed -n 's/^Version *: //p' | head -n 1; }
    # "S | Name | Type | Version | Arch | Repository", packages only.
    available_versions() {
        english zypper --no-refresh -q search -s --match-exact -t package -- "$name" 2>/dev/null |
            grep '|' | cut -d'|' -f4 | tr -d ' ' | grep -v '^Version$' | uniq_words
    }
}
use_brew() {
    # As root (the furnish service on macOS): brew runs as the account that owns Homebrew, if
    # that is safe; see furnish-homebrew.sh. Never brew as root, and not from PATH.
    . "$(dirname "$0")/furnish-homebrew.sh"
    if [ "$(id -u)" = 0 ]; then
        [ "$(uname -s)" = Darwin ] || return 1
        rc=0
        brew_owner || rc=$?
        [ "$rc" = 1 ] && return 1
        if [ "$rc" != 0 ]; then
            # A sudo rule from `furnish service install --homebrew-sudo` would now let whoever
            # can change Homebrew become root: remove it.
            rm -f /etc/sudoers.d/furnish-homebrew
            missing "Homebrew that only administrators can change ($brew_problem; see \"macOS: the service and Homebrew\" in furnish's admin guide)"
        fi
        cd / # the owner may not be allowed into the service's working folder
    else
        # Also where it is installed when PATH lacks it (apps started from the Finder, and users
        # whose Homebrew the service manages): status checks only read.
        if ! have brew; then
            for d in /opt/homebrew/bin /usr/local/bin; do [ -x "$d/brew" ] && PATH="$d:$PATH" && break; done
            have brew || return 1
        fi
    fi
    manager=brew via=Homebrew admin=false # Homebrew refuses to run as root
    # A cask (an app such as tunnelblick) is listed only with --cask when it is named.
    installed_version() {
        { run_brew command list --versions "$name" || run_brew command list --cask --versions "$name"; } 2>/dev/null | head -n 1 | cut -d' ' -f2
    }
    # Someone else's Homebrew (a user whose apps the service installs): brew fails for them, as
    # it can't write its caches and locks there. Its folders tell what is installed: opt/<name>
    # links to Cellar/<formula>/<version> (also for a formula's other names), Caskroom/<cask>/
    # holds a folder per version.
    if [ -z "${brew_user:-}" ] && prefix=$(brew --prefix 2>/dev/null) && [ -d "$prefix/Cellar" ] && [ ! -w "$prefix" ]; then
        installed_version() {
            v=$(readlink "$prefix/opt/$name" 2>/dev/null) && { echo "${v##*/}"; return; }
            ls -1 "$prefix/Caskroom/$name" 2>/dev/null | grep -v '^\.' | tail -n 1
        }
    fi
    do_install() { run_brew run_step install "$name"; }
    do_remove() { run_brew run_step uninstall "$name"; }
    install_desc="brew install $name" remove_desc="brew uninstall $name"
    run_as=${brew_user:-}
    # "name (1.0) < 1.1"; no auto-update of Homebrew itself during a status check.
    # brew outdated exits non-zero exactly when something is outdated.
    newer_version() { line=$(HOMEBREW_NO_AUTO_UPDATE=1 run_brew command outdated --verbose "$name" 2>/dev/null || true); [ -n "$line" ] && echo "${line##* }"; }
    do_upgrade() { run_brew run_step upgrade "$name"; }
    upgrade_desc="brew upgrade $name"
    scheme=brew
    # Installed: the update brew outdated names, else the installed version (brew's form, with a
    # revision such as 1.0_1). Else the formula's stable version or the cask's version.
    latest_version() {
        v=$(installed_version)
        if [ -n "$v" ] && [ "$v" != '?' ]; then
            n=$(newer_version) || n=
            echo "${n:-$v}"
            return
        fi
        info=$(HOMEBREW_NO_AUTO_UPDATE=1 run_brew command info --json=v2 "$name" 2>/dev/null | tr -d ' \n') || return 0
        case $info in
        *'"formulae":[{'*)
            v=${info#*'"versions":{"stable":"'} v=${v%%'"'*}
            r=${info#*'"revision":'} r=${r%%[!0-9]*}
            [ -z "$r" ] || [ "$r" = 0 ] || v="${v}_$r"
            [ -n "$v" ] && [ "${v#\{}" = "$v" ] && echo "$v"
            ;;
        *'"casks":[{'*)
            v=${info#*'"casks":[{'} v=${v#*'"version":"'} v=${v%%'"'*}
            [ -n "$v" ] && echo "$v"
            ;;
        esac
    }
}
# Homebrew's cask token can differ entirely from the installed .app's name. Get the
# app artifacts for the package brew would install (a formula can share a cask's token).
# Read the bundle in place. Do not execute its contents or any path from the metadata.
brew_external_bundle() {
    HOMEBREW_NO_AUTO_UPDATE=1 run_brew command info --json=v2 "$name" 2>/dev/null |
        /usr/bin/perl -MJSON::PP -0777 -e '
            my $info = eval { decode_json(<STDIN>) } or exit 1;
            exit 1 if @{$info->{formulae} // []};
            for my $artifact (@{$info->{casks}[0]{artifacts} // []}) {
                my $app = $artifact->{app} or next;
                # Older brew releases include a renamed target in the app arguments.
                my $renamed = ref($app->[1]) eq "HASH" ? $app->[1]{target} : undef;
                my $bundle = $artifact->{target} // $renamed;
                if (defined $bundle) {
                    # Keep subdirectories of Applications when the cask names one.
                    $bundle =~ s{\A(?:\Q$ENV{HOME}\E)?/Applications/}{};
                } else {
                    ($bundle) = $app->[0] =~ m{([^/]+)\z};
                }
                next unless defined $bundle && $bundle =~ /\.app\z/;
                next if $bundle =~ m{\A/|(?:\A|/)\.\.?(?:/|\z)|[\x00-\x1f]};
                utf8::encode($bundle);
                for my $dir ("/Applications", "$ENV{HOME}/Applications") {
                    my $path = "$dir/$bundle";
                    if (-f "$path/Contents/Info.plist") { print $path; exit 0 }
                }
            }
            exit 1;
        '
}
use_nix() {
    # Nix profiles, no root needed: the user's own profile. As root (the furnish service), the
    # shared profile /nix/var/nix/profiles/furnish, which the NixOS module puts on everyone's PATH.
    # A package declared in the NixOS or Home Manager configuration counts as installed; furnish
    # neither installs it again nor removes or updates it (see furnish-nix.sh).
    have nix || return 1
    . "$(dirname "$0")/furnish-nix.sh"
    manager=nix via=nix admin=false declared_in=
    # Add to the user's settings rather than replacing them.
    export NIX_CONFIG="${NIX_CONFIG:+$NIX_CONFIG
}extra-experimental-features = nix-command flakes"
    shared=/nix/var/nix/profiles/furnish
    ref=${nixpkgs:-nixpkgs}
    if [ "$(id -u)" = 0 ]; then profile="--profile $shared"; else profile=""; fi
    # The declarative roots don't change while furnish runs: look once (needs the final $name).
    prepare() {
        nix_lookup "$name"
        # The configuration's folder, as furnish found it (FURNISH_DETECTED__*, see furnish-nix.sh).
        declared_in=$(nix_configuration "$declared_by")
        case $declared_by in
        nixos) cant_remove="$subject is in your NixOS configuration${declared_in:+ ($declared_in)}: remove it there" ;;
        home-manager) cant_remove="$subject is in your Home Manager configuration${declared_in:+ ($declared_in)}: remove it there" ;;
        esac
    }
    # Where it comes from: nixos, home-manager or profile (nix profile, the user's or the shared one).
    extra_state() {
        if [ -n "$declared_by" ]; then
            printf ',"managedBy":"%s"' "$declared_by"
            [ -z "$declared_in" ] || printf ',%s' "$(json_member declaredIn "$declared_in")"
        elif [ -n "$1" ]; then printf ',"managedBy":"profile"'
        fi
    }
    # A profile names an entry after the package's last attribute: kdePackages.kdenlive → kdenlive.
    entry=${name##*.}
    # The version in a profile ("" if it doesn't hold the package).
    version_in() {
        nix profile list $1 --json 2>/dev/null | tr '{,' '\n\n' | grep -q "^\"$entry\":" || return 0
        path=$(nix profile list $1 --json | grep -o "/nix/store/[a-z0-9]*-$entry-[^\"]*" | head -n1)
        echo "${path##*-"$entry"-}"
    }
    # Declared, or installed for this user or for everyone.
    installed_version() {
        [ -n "$declared_by" ] && { echo "$declared_version"; return; }
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
    # The version in the referenced nixpkgs, if it differs from the installed one. None for a
    # declared package: it moves with the configuration's inputs.
    newer_version() {
        [ -z "$declared_by" ] || return 0
        available=$(nix_available) || return 0
        [ -n "$available" ] && [ "$available" != "$(installed_version)" ] && echo "$available"
    }
    # A declared package's version moves with the configuration, which furnish doesn't change.
    scheme=nix
    latest_version() { [ -n "$declared_by" ] || nix_available; }
    # The version in the referenced nixpkgs.
    nix_available() {
        # Offline, say, the check fails: the app counts as up to date, and the activity log says why.
        # Errors go to a private file (mktemp: random name, only ours), warnings on success are dropped.
        err=$(mktemp) || return 0
        if ! available=$(nix eval --raw "$ref#$name.version" 2>"$err"); then
            echo "{\"warn\":$(json "couldn't check $ref for a newer $name: $(tail -n1 "$err")")}" >&2
            rm -f "$err"
            return 0
        fi
        rm -f "$err"
        echo "$available"
    }
}

# The program $command, where people's installers put programs: PATH, then the folders an app
# started from the Finder or a desktop menu lacks on its PATH (~/.local/bin for vendors'
# installers, Homebrew's, where npm and pip put programs with Homebrew's Node and Python). Only
# looks: never runs it, so a folder others can write to is harmless.
find_program() {
    rest=$PATH:$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin
    while [ -n "$rest" ]; do
        d=${rest%%:*}
        case $rest in *:*) rest=${rest#*:} ;; *) rest= ;; esac
        case $d in /*) ;; *) continue ;; esac # not relative to wherever DSC runs it
        [ -f "$d/$1" ] && [ -x "$d/$1" ] && { echo "$d/$1"; return 0; }
    done
    return 1
}
case ${command:-} in */*|-*) fail 10 "command must be a program's name, without a folder: $command" ;; esac

need=
case "${packageManager:-}" in
pacman) use_pacman || need="pacman" ;;
apt) use_apt || need="apt (Debian, Ubuntu)" ;;
dnf) use_dnf || need="dnf (Fedora, RHEL)" ;;
zypper) use_zypper || need="zypper (openSUSE)" ;;
brew) use_brew || need="Homebrew (brew)" ;;
nix) use_nix || need="Nix" ;;
'')
    if [ "$(uname -s)" = Darwin ]; then need="Homebrew (brew)"
    else need="a package manager (pacman, apt, dnf, zypper or nix)"; fi
    ;;
*) fail 10 "unknown packageManager '$packageManager' (pacman, apt, dnf, zypper, brew, nix)" ;;
esac

if [ -n "$need" ]; then
    # Without the package manager the program alone tells whether it is installed (see below).
    if [ -z "${command:-}" ] || ! find_program "$command" >/dev/null; then missing "$need"; fi
    via=$need admin=false
    installed_version() { :; }
    install_desc="(needs $need)" remove_desc=$install_desc
fi
# Not an option to the package manager (the schema says so too; this holds without it).
case $name in -*) fail 10 "package name starts with '-': $name" ;; esac
case ${nixpkgs:-} in -*) fail 10 "nixpkgs starts with '-': $nixpkgs" ;; esac

# Installed some other way (npm, the vendor's installer, another package manager): the program is
# there although the package manager doesn't have the package. It counts as installed, not by
# furnish, which neither installs it again nor updates or removes it.
check_external_install() {
    [ -z "$version" ] || return 0
    if [ -n "${command:-}" ] && outside=$(find_program "$command"); then :
    elif [ "${manager:-}" = brew ] && [ "$(uname -s)" = Darwin ] && outside=$(brew_external_bundle); then :
    else return 0
    fi
    version='?'
    installed_version() { echo '?'; }
    newer_version() { :; }
    extra_state() { printf ',"managedBy":"other",%s' "$(json_member path "$outside")"; }
    cant_remove="$subject was installed outside furnish ($outside): uninstall it the way it was installed"
}

furnish_main "$@"
