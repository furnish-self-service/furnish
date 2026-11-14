#!/usr/bin/env bash
# End-to-end check: add a collection, list, install, re-install, uninstall.
#
#   tests/e2e.sh [--real] [path/to/furnish]
#
# Default: a fake pacman (and sudo/pkexec) on PATH, so it needs no root and changes nothing.
# --real:  installs and removes hyperfine with the machine's real package manager (CI).
set -euo pipefail
real=false
if [ "${1:-}" = --real ]; then real=true; shift; fi
furnish=$(realpath "${1:-target/fast/furnish}")
fixture=$(realpath "$(dirname "$0")/fixtures/collection")
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
# Git Bash on Windows: give the native furnish.exe a Windows path.
if command -v cygpath >/dev/null; then tmp=$(cygpath -m "$tmp"); fi
export FURNISH_DATA_DIR="$tmp/data"

if ! $real; then
    # Fake package manager: installed packages are files "<name>" containing the version.
    mkdir -p "$tmp/bin" "$tmp/pkgs"
    cat > "$tmp/bin/pacman" <<SH
#!/bin/sh
case "\$1" in
  -Q) [ -f "$tmp/pkgs/\$2" ] && echo "\$2 \$(cat "$tmp/pkgs/\$2")" ;;
  -S) echo 1.0 > "$tmp/pkgs/\$4" ;;
  -R) rm "$tmp/pkgs/\$3" ;;
esac
SH
    printf '#!/bin/sh\nexec "$@"\n' > "$tmp/bin/pkexec"
    cp "$tmp/bin/pkexec" "$tmp/bin/sudo"
    chmod +x "$tmp/bin/"*
    export PATH="$tmp/bin:$PATH"
fi

# Collections are git repos: turn the fixture into one.
cp -r "$fixture" "$tmp/repo"

# Real installer files for whatever this platform supports (Furnish/Installer). Each line of
# $tmp/installers: file name, then extra YAML properties.
mkdir -p "$tmp/repo/installers"
: > "$tmp/installers"
if $real; then
    if command -v dpkg-deb >/dev/null; then
        d="$tmp/deb"; mkdir -p "$d/DEBIAN" "$d/opt/furnish-demo"; echo demo > "$d/opt/furnish-demo/demo.txt"
        printf 'Package: furnish-demo\nVersion: 1.0\nArchitecture: all\nMaintainer: furnish <noreply@example.com>\nDescription: furnish test package\n' > "$d/DEBIAN/control"
        dpkg-deb --build --root-owner-group "$d" "$tmp/repo/installers/furnish-demo.deb" >/dev/null
        echo "furnish-demo.deb|" >> "$tmp/installers"
    elif command -v rpmbuild >/dev/null; then
        cat > "$tmp/demo.spec" <<'SPEC'
Name: furnish-demo
Version: 1.0
Release: 1
Summary: furnish test package
License: MIT
BuildArch: noarch
%description
furnish test package
%install
mkdir -p %{buildroot}/opt/furnish-demo && echo demo > %{buildroot}/opt/furnish-demo/demo.txt
%files
/opt/furnish-demo/demo.txt
SPEC
        rpmbuild -bb --quiet --define "_topdir $tmp/rpm" "$tmp/demo.spec" >/dev/null 2>&1
        cp "$tmp"/rpm/RPMS/noarch/*.rpm "$tmp/repo/installers/furnish-demo.rpm"
        echo "furnish-demo.rpm|" >> "$tmp/installers"
    fi
    if command -v pkgbuild >/dev/null; then
        mkdir -p "$tmp/pkgroot/opt/furnish-demo" && echo demo > "$tmp/pkgroot/opt/furnish-demo/demo.txt"
        pkgbuild --quiet --root "$tmp/pkgroot" --identifier io.github.furnish-self-service.demo --version 1.0 \
            --install-location / "$tmp/repo/installers/furnish-demo.pkg"
        echo "furnish-demo.pkg|      packageId: io.github.furnish-self-service.demo" >> "$tmp/installers"
        app="$tmp/dmgsrc/FurnishDemo.app/Contents"; mkdir -p "$app"
        printf '<?xml version="1.0" encoding="UTF-8"?>\n<plist version="1.0"><dict><key>CFBundleShortVersionString</key><string>1.0</string></dict></plist>\n' > "$app/Info.plist"
        hdiutil create -quiet -volname FurnishDemo -srcfolder "$tmp/dmgsrc" -ov -format UDZO "$tmp/repo/installers/furnish-demo.dmg"
        echo "furnish-demo.dmg|      app: FurnishDemo.app" >> "$tmp/installers"
    fi
    if [ -n "${FURNISH_TEST_MSI:-}" ]; then  # built by CI with WiX (tests/fixtures/installers/demo.wxs)
        cp "$FURNISH_TEST_MSI" "$tmp/repo/installers/furnish-demo.msi"
        echo "furnish-demo.msi|" >> "$tmp/installers"
    fi
fi
n=0
while IFS='|' read -r file props; do
    n=$((n + 1))
    { printf 'metadata:\n  furnish:\n    description: Installer test\n    categories: [Test]\nresources:\n'
      printf '  - name: demo installer\n    type: Furnish/Installer\n    properties:\n      file: installers/%s\n' "$file"
      [ -n "$props" ] && printf '%s\n' "$props"; } > "$tmp/repo/apps/installer-$n.dsc.yaml"
done < "$tmp/installers"

git -C "$tmp/repo" init -q
git -C "$tmp/repo" add -A
git -C "$tmp/repo" -c user.name=t -c user.email=t@t commit -qm init

# Echo each command and its output to stderr (logs), return the output for checks.
# No `tee /dev/stderr`: Git Bash on Windows has no /dev/stderr.
step() {
    echo "+ furnish $*" >&2
    local out rc=0
    out=$("$furnish" "$@") || rc=$?
    echo "$out" >&2
    echo "$out"
    return $rc
}
expect() { grep -q -- "$2" <<<"$1" || { echo "FAIL: expected '$2'" >&2; exit 1; }; }

step collection add "$tmp/repo" --name fixture >/dev/null
expect "$(step list)" "·  hyperfine"
expect "$(step list)" "–  nowhere"
if step install nowhere >/dev/null 2>&1; then echo "FAIL: installing an unavailable app succeeded" >&2; exit 1; fi
expect "$(step install hyperfine --dry-run)" "+ install"
expect "$(step list)" "·  hyperfine"   # a preview changes nothing
expect "$(step install hyperfine)" "Installed hyperfine"
expect "$(step list)" "✓  hyperfine"
expect "$(step install fixture/hyperfine)" "already installed"
expect "$(step uninstall hyperfine --dry-run)" "- remove"
expect "$(step list)" "✓  hyperfine"
expect "$(step uninstall hyperfine)" "Uninstalled hyperfine"
expect "$(step list)" "·  hyperfine"
for i in $(seq 1 "$n"); do
    app="installer-$i"
    echo "--- $app: $(sed -n "${i}p" "$tmp/installers" | cut -d'|' -f1)" >&2
    expect "$(step list)" "·  $app "
    expect "$(step install "$app" --dry-run)" "+ install"
    expect "$(step install "$app")" "Installed $app"
    expect "$(step list)" "✓  $app "
    expect "$(step uninstall "$app" --dry-run)" "- remove"
    expect "$(step uninstall "$app")" "Uninstalled $app"
    expect "$(step list)" "·  $app "
done
[ "$n" -gt 0 ] || echo "(no installer format can be built on this platform: installer test skipped)" >&2
step collection remove fixture >/dev/null

# Git LFS: a collection whose files are stored with LFS gets the real files, not pointers.
if git lfs version >/dev/null 2>&1; then
    git init -q "$tmp/lfs-src"
    git -C "$tmp/lfs-src" lfs install --local >/dev/null
    git -C "$tmp/lfs-src" lfs track '*.bin' >/dev/null
    echo "real installer bytes" > "$tmp/lfs-src/payload.bin"
    git -C "$tmp/lfs-src" add -A && git -C "$tmp/lfs-src" -c user.name=t -c user.email=t@t commit -qm lfs
    git init -q --bare "$tmp/lfs.git"
    branch=$(git -C "$tmp/lfs-src" symbolic-ref --short HEAD)
    git -C "$tmp/lfs.git" symbolic-ref HEAD "refs/heads/$branch"
    git -C "$tmp/lfs-src" push -q "file://$tmp/lfs.git" "$branch" 2>/dev/null
    step collection add "file://$tmp/lfs.git" --name lfs >/dev/null
    grep -q "real installer bytes" "$FURNISH_DATA_DIR/collections/lfs/payload.bin" || { echo "FAIL: LFS file not downloaded" >&2; exit 1; }
    step collection remove lfs >/dev/null
    lfs="with LFS"
else
    lfs="LFS skipped: git-lfs not installed"
fi
echo "e2e OK ($lfs, $n installer(s))" >&2
echo "e2e OK ($($real && echo real || echo fake) package manager)"
