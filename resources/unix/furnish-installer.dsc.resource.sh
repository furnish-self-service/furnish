#!/bin/sh
# Furnish/Installer: install an installer file shipped in the collection (Linux: .deb, .rpm;
# macOS: .pkg, .dmg). furnish passes `file` as an absolute path inside the collection.
# DSC passes the resource properties as env vars: $file, $packageId (.pkg), $app (.dmg), $_exist.
set -eu
. "$(dirname "$0")/furnish-common.sh"

[ -n "${file:-}" ] || fail 10 "property file is required"
# Echo back every identifying property: DSC's test compares each desired property with the state.
identity=$(json_member file "$file")
[ -n "${packageId:-}" ] && identity="$identity,$(json_member packageId "$packageId")"
[ -n "${app:-}" ] && identity="$identity,$(json_member app "$app")"
subject=$(basename "$file")

[ -f "$file" ] || fail 10 "installer not found: $file"
# Without git-lfs, an LFS-tracked file is a small text pointer instead of the installer.
if head -c 64 "$file" | grep -q '^version https://git-lfs'; then
    missing "git-lfs to download $subject (install it, then update the collection)"
fi

rpm_version() { v=$(rpm -q --qf '%{VERSION}-%{RELEASE}' "$1" 2>/dev/null) && echo "$v"; }

admin=true
case "$file" in
*.deb)
    command -v dpkg-deb >/dev/null 2>&1 || missing "dpkg (Debian, Ubuntu) for .deb installers"
    id=$(dpkg-deb -f "$file" Package)
    via=apt
    installed_version() { dpkg-query -W -f='${db:Status-Status} ${Version}' "$id" 2>/dev/null | sed -n 's/^installed //p'; }
    do_install() { run_step env DEBIAN_FRONTEND=noninteractive apt-get install -y "$file"; }
    do_remove() { run_step env DEBIAN_FRONTEND=noninteractive apt-get remove -y "$id"; }
    install_desc="apt-get install -y $file" remove_desc="apt-get remove -y $id"
    # A different version in the collection's file is an update: installing the file brings it.
    newer_version() { v=$(dpkg-deb -f "$file" Version) && [ "$v" != "$(installed_version)" ] && echo "$v"; }
    ;;
*.rpm)
    command -v rpm >/dev/null 2>&1 || missing "rpm (Fedora, openSUSE, RHEL) for .rpm installers"
    id=$(rpm -qp --qf '%{NAME}' "$file" 2>/dev/null)
    installed_version() { rpm_version "$id"; }
    newer_version() { v=$(rpm -qp --qf '%{VERSION}-%{RELEASE}' "$file" 2>/dev/null) && [ "$v" != "$(installed_version)" ] && echo "$v"; }
    if command -v dnf >/dev/null 2>&1; then
        via=dnf
        do_install() { run_step dnf install -y "$file"; }
        do_remove() { run_step dnf remove -y "$id"; }
        install_desc="dnf install -y $file" remove_desc="dnf remove -y $id"
    elif command -v zypper >/dev/null 2>&1; then
        via=zypper
        do_install() { run_step zypper --non-interactive install --allow-unsigned-rpm "$file"; }
        do_remove() { run_step zypper --non-interactive remove "$id"; }
        install_desc="zypper install $file" remove_desc="zypper remove $id"
    else
        via=rpm
        do_install() { run_step rpm -U "$file"; }
        do_remove() { run_step rpm -e "$id"; }
        install_desc="rpm -U $file" remove_desc="rpm -e $id"
    fi
    ;;
*.pkg)
    command -v installer >/dev/null 2>&1 || missing "macOS for .pkg installers"
    [ -n "${packageId:-}" ] || fail 10 ".pkg installers need the property packageId (see: pkgutil --pkgs)"
    via=installer
    installed_version() { pkgutil --pkg-info "$packageId" 2>/dev/null | sed -n 's/^version: //p'; }
    do_install() { run_step installer -pkg "$file" -target /; }
    # macOS has no uninstaller for .pkg: remove the files it installed, then its receipt.
    do_remove() { run_step sh -c 'pkgutil --only-files --files "$1" | (cd / && tr "\n" "\0" | xargs -0 rm -f) && pkgutil --forget "$1"' sh "$packageId"; }
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
