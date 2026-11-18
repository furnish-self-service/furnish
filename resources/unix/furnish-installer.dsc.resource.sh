#!/bin/sh
# Furnish/Installer: install a collection file or a SHA-256-pinned download (Linux: .deb, .rpm;
# macOS: .pkg, .dmg). furnish passes `file` as an absolute path inside the collection.
# DSC passes the resource properties as env vars: $file, $packageId (.pkg), $app (.dmg), $_exist,
# and from furnish's required versions $reportVersions and $targetVersion (see furnish-common.sh):
# the only version an installer can install is the one in the file.
set -eu
. "$(dirname "$0")/furnish-common.sh"

# DSC compares each desired property with the returned state. Keep the collection's
# identity even when an installer is downloaded into furnish's content-addressed cache.
if [ -n "${url:-}" ]; then
    [ -z "${file:-}" ] && [ -n "${sha256:-}" ] || fail 10 "use either file or url with sha256"
    file=$("$FURNISH_EXE" dsc-resource installer-download lookup)
    verified=false
    verify_download() {
        if ! "$verified"; then
            file=$("$FURNISH_EXE" dsc-resource installer-download fetch) || exit $?
            verified=true
        fi
    }
    if [ "${1:-}" = set ] && [ "${2:-}" != --what-if ]; then
        # An uncached payload cannot identify a package. An explicitly supplied command
        # can still establish an external installation without fetching or running it.
        if [ "${_exist:-true}" = true ] && ! find_external_command >/dev/null; then
            verify_download
        elif [ -f "$file" ]; then
            # An inferred identity used to change the machine must come from pinned bytes.
            case "$file" in
            *.dmg) ;;
            *.pkg) [ -n "${packageId:-}" ] || verify_download ;;
            *) verify_download ;;
            esac
        fi
    fi
    identity="$(json_member url "$url"),$(json_member sha256 "$sha256")"
    path=${url%%\?*}; subject=${path##*/}
else
    [ -n "${file:-}" ] && [ -z "${sha256:-}" ] || fail 10 "use either file or url with sha256"
    identity=$(json_member file "$file")
    subject=$(basename "$file")
fi
[ -n "${command:-}" ] && identity="$identity,$(json_member command "$command")"
[ -n "${packageId:-}" ] && identity="$identity,$(json_member packageId "$packageId")"
[ -n "${app:-}" ] && identity="$identity,$(json_member app "$app")"
case "$file" in
*.deb) scheme=dpkg ;;
*.rpm) scheme=rpm ;;
*) scheme=generic ;;
esac

# A link could point anywhere outside the collection; test -L also holds for a dangling one.
[ -L "$file" ] && fail 10 "installer is a symbolic link: $file"
# An explicit receipt ID or app bundle identifies a remote macOS installation even
# before the installer is cached, including for offline status and removal.
known_without_file=false
if [ -n "${url:-}" ] || find_external_command >/dev/null; then
    case "$file" in
    *.pkg) [ -z "${packageId:-}" ] || known_without_file=true ;;
    *.dmg) [ -z "${app:-}" ] || known_without_file=true ;;
    esac
fi
if [ ! -f "$file" ] && ! "$known_without_file"; then
    # Left out of the checkout (blobsOnDemand): furnish downloads it when the app is installed.
    # Until then the app counts as not installed, and only an actual install needs the file.
    if [ "${1:-}" = set ] && [ "${2:-}" != --what-if ] && { [ -z "${url:-}" ] || [ "${_exist:-true}" = true ]; } && ! find_external_command >/dev/null; then
        fail 10 "installer not found: $file"
    fi
    installed_version() { :; }
    admin=true via="its installer"
    if [ -n "${url:-}" ]; then install_desc="download $subject and verify its SHA-256, then install it"
    else install_desc="download $subject from the collection, then install it"; fi
    furnish_main "$@"
    exit
fi
# Without git-lfs, an LFS-tracked file is a small text pointer instead of the installer.
if [ -z "${url:-}" ] && [ -f "$file" ] && head -c 64 "$file" | grep -q '^version https://git-lfs'; then
    missing "git-lfs to download $subject (install it, then update the collection)"
fi

# The epoch only when it isn't 0, as dnf and zypper show versions.
rpm_qf='%|EPOCH?{%{EPOCH}:}:{}|%{VERSION}-%{RELEASE}'
rpm_version() { v=$(rpm -q --qf "$rpm_qf" -- "$1" 2>/dev/null) && echo "$v"; }

admin=true
case "$file" in
*.deb)
    command -v dpkg-deb >/dev/null 2>&1 || missing "dpkg (Debian, Ubuntu) for .deb installers"
    id=$(dpkg-deb -f -- "$file" Package)
    via=apt
    installed_version() { dpkg-query -W -f='${db:Status-Status} ${Version}' -- "$id" 2>/dev/null | sed -n 's/^installed //p'; }
    # --allow-downgrades: the collection's file is the version wanted, also an earlier one.
    do_install() { run_step env DEBIAN_FRONTEND=noninteractive apt-get install -y --allow-downgrades -- "$file"; }
    do_remove() { run_step env DEBIAN_FRONTEND=noninteractive apt-get remove -y -- "$id"; }
    install_desc="apt-get install -y --allow-downgrades $file" remove_desc="apt-get remove -y $id"
    # A different version in the collection's file is an update: installing the file brings it.
    latest_version() { dpkg-deb -f -- "$file" Version; }
    newer_version() { v=$(latest_version) && [ "$v" != "$(installed_version)" ] && echo "$v"; }
    ;;
*.rpm)
    command -v rpm >/dev/null 2>&1 || missing "rpm (Fedora, openSUSE, RHEL) for .rpm installers"
    id=$(rpm -qp --qf '%{NAME}' -- "$file" 2>/dev/null)
    installed_version() { rpm_version "$id"; }
    latest_version() { rpm -qp --qf "$rpm_qf" -- "$file" 2>/dev/null; }
    newer_version() { v=$(latest_version) && [ "$v" != "$(installed_version)" ] && echo "$v"; }
    if command -v dnf >/dev/null 2>&1; then
        via=dnf
        do_install() { run_step dnf install -y -- "$file"; }
        do_remove() { run_step dnf remove -y -- "$id"; }
        install_desc="dnf install -y $file" remove_desc="dnf remove -y $id"
    elif command -v zypper >/dev/null 2>&1; then
        via=zypper
        do_install() { run_step zypper --non-interactive install --allow-unsigned-rpm --oldpackage -- "$file"; }
        do_remove() { run_step zypper --non-interactive remove -- "$id"; }
        install_desc="zypper install $file" remove_desc="zypper remove $id"
    else
        via=rpm
        do_install() { run_step rpm -U --oldpackage -- "$file"; }
        do_remove() { run_step rpm -e -- "$id"; }
        install_desc="rpm -U --oldpackage $file" remove_desc="rpm -e $id"
    fi
    ;;
*.pkg)
    command -v installer >/dev/null 2>&1 || missing "macOS for .pkg installers"
    if [ -n "${url:-}" ] && [ -f "$file" ] && { [ "${_exist:-true}" = true ] || [ -z "${packageId:-}" ]; }; then
        # macOS tar reads flat package archives. A productbuild distribution stores
        # a PackageInfo inside each component; xmllint accepts exactly one document.
        metadata=$(tar -xOf "$file" '*PackageInfo' 2>/dev/null |
            xmllint --nonet --xpath 'concat(/pkg-info/@identifier,"|",/pkg-info/@version)' - 2>/dev/null) || metadata=
        inferred=${metadata%%|*}
        case $inferred in ''|-*|*[!A-Za-z0-9._-]*) inferred= ;; esac
        if [ -z "${packageId:-}" ]; then
            [ -n "$inferred" ] || fail 10 "set packageId for $subject: no single package receipt could be identified"
            packageId=$inferred
        fi
        if [ "$packageId" = "$inferred" ]; then
            pkg_version=${metadata#*|}
            latest_version() { echo "$pkg_version"; }
            newer_version() { [ -n "$pkg_version" ] && [ "$pkg_version" != "$(installed_version)" ] && echo "$pkg_version"; }
        fi
    fi
    [ -n "${packageId:-}" ] || fail 10 ".pkg installers need the property packageId (see: pkgutil --pkgs)"
    via=installer
    installed_version() { pkgutil --pkg-info "$packageId" 2>/dev/null | sed -n 's/^version: //p'; }
    do_install() { [ -z "${url:-}" ] || verify_download; run_step installer -pkg "$file" -target /; }
    # pkgutil lists paths relative to the receipt's install location (Homebrew: /opt/homebrew).
    # Unlink one leaf from its checked physical directory, so symlinked parents cannot redirect it.
    do_remove() {
        run_step /usr/bin/perl -MCwd=getcwd -e '
            use strict;
            use warnings;
            my $id = shift;
            my $tool = "/usr/sbin/pkgutil";
            $ENV{LC_ALL} = "C";
            open my $info, "-|", $tool, "--pkg-info", $id or die "reading receipt: $!";
            my ($volume, $location);
            while (<$info>) {
                chomp;
                $volume = $1 if /^volume: (.*)$/;
                $location = $1 if /^location: (.*)$/;
            }
            close $info or die "could not read receipt for $id";
            die "invalid receipt location for $id" unless defined $volume && $volume eq "/" && defined $location;
            my @dirs = grep { length } split m{/}, $location;
            die "invalid receipt location for $id" if grep { /[[:cntrl:]]/ || $_ eq ".." || $_ eq "." } @dirs;
            my $root = "/" . join("/", @dirs);
            chdir $root and getcwd() eq $root or die "cannot verify receipt directory $root";
            open my $list, "-|", $tool, "--only-files", "--files", $id or die "listing receipt files: $!";
            my @files = <$list>;
            close $list or die "could not list receipt files for $id";
            chomp @files;
            for (@files) {
                s{^(?:\./)+}{};
                die "invalid path in receipt for $id" if !length || m{^/|(?:^|/)\.{1,2}(?:/|$)|[[:cntrl:]]};
            }
            for my $entry (@files) {
                my ($dir, $leaf) = $entry =~ m{^(?:(.*)/)?([^/]+)$} or die "invalid receipt entry";
                my $parent = defined $dir ? ($root eq "/" ? "/" : "$root/") . $dir : $root;
                next unless -d $parent;
                chdir $parent and getcwd() eq $parent or die "cannot verify receipt directory $parent";
                unlink "./$leaf" or die "removing $entry: $!" if -e "./$leaf" || -l "./$leaf";
            }
            exec $tool, "--forget", $id or die "forgetting $id: $!";
        ' "$packageId"
    }
    install_desc="installer -pkg $file -target /" remove_desc="remove the files of $packageId, then pkgutil --forget $packageId"
    ;;
*.dmg)
    command -v hdiutil >/dev/null 2>&1 || missing "macOS for .dmg installers"
    [ -n "${app:-}" ] || fail 10 ".dmg installers need the property app, e.g. Firefox.app"
    via="the disk image"
    admin=false # /Applications is writable for admin users
    target="/Applications/$app"
    installed_version() {
        [ -d "$target" ] || return 0
        defaults read "$target/Contents/Info" CFBundleShortVersionString 2>/dev/null || echo '?'
    }
    do_install() {
        [ -z "${url:-}" ] || verify_download
        run_step sh -c 'mnt=$(mktemp -d) && hdiutil attach -nobrowse -readonly -mountpoint "$mnt" "$1" >/dev/null &&
            cp -R "$mnt/$2" /Applications/; rc=$?; hdiutil detach "$mnt" >/dev/null; exit $rc' sh "$file" "$app"
    }
    do_remove() { run_step rm -rf "$target"; }
    install_desc="copy $app from $subject to /Applications" remove_desc="rm -rf $target"
    ;;
*)
    fail 10 "unsupported installer: $subject (Linux: .deb, .rpm; macOS: .pkg, .dmg; Windows: .msi, .exe)"
    ;;
esac

furnish_main "$@"
