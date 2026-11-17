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
for a do shift; [ "\$a" = -- ] || set -- "\$@" "\$a"; done  # drop the end of options
case "\$1" in
  -Q) [ -f "$tmp/pkgs/\$2" ] && echo "\$2 \$(cat "$tmp/pkgs/\$2")" ;;
  -Qu) [ -f "$tmp/pkgs/\$2.new" ] && echo "\$2 \$(cat "$tmp/pkgs/\$2") -> \$(cat "$tmp/pkgs/\$2.new")" ;;
  -S) if [ -f "$tmp/pkgs/\$4.new" ]; then mv "$tmp/pkgs/\$4.new" "$tmp/pkgs/\$4"; else echo 1.0 > "$tmp/pkgs/\$4"; fi; if [ "\$4" = brew ]; then cp "$tmp/fake-brew" "$tmp/bin/brew"; fi ;;
  -R) rm "$tmp/pkgs/\$3"; if [ "\$3" = brew ]; then rm -f "$tmp/bin/brew"; fi ;;
esac
SH
    # Installing the "brew" package provides this fake Homebrew (see the homebrew fixture app).
    cat > "$tmp/fake-brew" <<SH
#!/bin/sh
mkdir -p "$tmp/brew-pkgs"
case "\$1" in
  list) [ -f "$tmp/brew-pkgs/\$3" ] && echo "\$3 1.0" ;;
  install) touch "$tmp/brew-pkgs/\$2" ;;
  uninstall) rm -f "$tmp/brew-pkgs/\$2" ;;
esac
SH
    chmod +x "$tmp/fake-brew"
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

# DSC's own resources, when they are beside furnish (scripts/fetch-dsc.mjs; the installers ship them).
dsc=false
if [ -d "$(dirname "$furnish")/dsc" ]; then
    dsc=true
    cat > "$tmp/repo/apps/dsc-file.dsc.yaml" <<YAML
metadata:
  furnish:
    description: A file from DSC's resources
    categories: [Test]
resources:
  - name: settings file
    type: Microsoft.Filesystem.File/Content
    properties:
      path: $tmp/dsc-file.txt
      content: furnish
YAML
    # The same with removal steps, run instead of setting File/Content to _exist: false.
    cat > "$tmp/repo/apps/dsc-removable.dsc.yaml" <<YAML
metadata:
  furnish:
    description: A file that removal steps delete
    categories: [Test]
    uninstall:
      - name: delete the file
        type: Microsoft.DSC.Transitional/RunCommandOnSet
        condition: "[not(equals(context().os.family, 'Windows'))]"
        properties: { executable: rm, arguments: [-f, "$tmp/dsc-removable.txt"] }
      - name: delete the file on Windows
        type: Microsoft.DSC.Transitional/RunCommandOnSet
        condition: "[equals(context().os.family, 'Windows')]"
        properties: { executable: powershell, arguments: [-NoProfile, -Command, "Remove-Item -LiteralPath '$tmp/dsc-removable.txt'"] }
resources:
  - name: settings file
    type: Microsoft.Filesystem.File/Content
    properties: { path: "$tmp/dsc-removable.txt", content: furnish }
YAML
    # Removal steps that leave the file: furnish must not report it uninstalled.
    cat > "$tmp/repo/apps/dsc-stays.dsc.yaml" <<YAML
metadata:
  furnish:
    description: Removal steps that remove nothing
    categories: [Test]
    uninstall:
      - name: do nothing
        type: Microsoft.DSC.Transitional/RunCommandOnSet
        properties: { executable: git, arguments: [--version] }
resources:
  - name: settings file
    type: Microsoft.Filesystem.File/Content
    properties: { path: "$tmp/dsc-stays.txt", content: furnish }
YAML
    # A command that fails: RunCommandOnSet exits 0 anyway and reports the exit code in its
    # result. furnish must stop there, before the file.
    cat > "$tmp/repo/apps/dsc-fails.dsc.yaml" <<YAML
metadata:
  furnish:
    description: A command that fails
    categories: [Test]
resources:
  - name: fail
    type: Microsoft.DSC.Transitional/RunCommandOnSet
    condition: "[not(equals(context().os.family, 'Windows'))]"
    properties: { executable: "false" }
  - name: fail on Windows
    type: Microsoft.DSC.Transitional/RunCommandOnSet
    condition: "[equals(context().os.family, 'Windows')]"
    properties: { executable: cmd, arguments: [/c, exit 1] }
  - name: settings file
    type: Microsoft.Filesystem.File/Content
    dependsOn: ["[resourceId('Microsoft.DSC.Transitional/RunCommandOnSet', 'fail')]",
                "[resourceId('Microsoft.DSC.Transitional/RunCommandOnSet', 'fail on Windows')]"]
    properties: { path: "$tmp/dsc-fails.txt", content: furnish }
YAML
    # Every step succeeds, but the last one deletes what the first wrote: the app isn't
    # installed afterwards, and furnish must say so.
    cat > "$tmp/repo/apps/dsc-differs.dsc.yaml" <<YAML
metadata:
  furnish:
    description: Steps that succeed without installing
    categories: [Test]
resources:
  - name: settings file
    type: Microsoft.Filesystem.File/Content
    properties: { path: "$tmp/dsc-differs.txt", content: furnish }
  - name: undo
    type: Microsoft.DSC.Transitional/RunCommandOnSet
    condition: "[not(equals(context().os.family, 'Windows'))]"
    dependsOn: ["[resourceId('Microsoft.Filesystem.File/Content', 'settings file')]"]
    properties: { executable: rm, arguments: [-f, "$tmp/dsc-differs.txt"] }
  - name: undo on Windows
    type: Microsoft.DSC.Transitional/RunCommandOnSet
    condition: "[equals(context().os.family, 'Windows')]"
    dependsOn: ["[resourceId('Microsoft.Filesystem.File/Content', 'settings file')]"]
    properties: { executable: powershell, arguments: [-NoProfile, -Command, "Remove-Item -LiteralPath '$tmp/dsc-differs.txt'"] }
YAML
fi

# furnish's configuration resources: one app for every system, no conditions needed.
mkdir -p "$tmp/repo/files" "$tmp/conf-seed"
printf 'remote vpn.fixture.example 1194\n' > "$tmp/repo/files/profile.conf"
home_file=".furnish-e2e-$$"
cat > "$tmp/repo/apps/config.dsc.yaml" <<YAML
metadata:
  furnish:
    description: Configuration resources
    categories: [Test]
resources:
  - name: folder
    type: Furnish/Directory
    properties: { path: "$tmp/conf", private: true }
  - name: profile
    type: Furnish/File
    properties: { path: "$tmp/conf/profile.conf", source: files/profile.conf, private: true }
  - name: include
    type: Furnish/Line
    properties: { path: "$tmp/conf/ssh_config", line: Include config.d/fixture.conf, before: '^\s*(Host|Match)\s' }
  - name: host
    type: Furnish/Block
    properties: { path: "$tmp/conf/ssh_config", content: "Host git.fixture.example\n  User git", backup: true }
  - name: plugin
    type: Furnish/Value
    properties: { path: "$tmp/conf/settings.json", location: "\$.enabledPlugins['fixture@market']", value: true }
  - name: home file
    type: Furnish/File
    properties: { path: "~/$home_file", content: "furnish" }
  - name: server
    type: Furnish/Command
    properties:
      test: { command: [git, config, --file, "$tmp/conf/gitconfig", fixture.server], output: https://fixture.example }
      set: [git, config, --file, "$tmp/conf/gitconfig", fixture.server, https://fixture.example]
      remove: [git, config, --file, "$tmp/conf/gitconfig", fixture.server, ""]  # succeeds also with nothing to remove
YAML

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

# The collection asks a required question; without a terminal furnish says how to answer it,
# and its apps wait for the answer.
expect "$("$furnish" collection add "$tmp/repo" --name fixture 2>&1)" "furnish option set fixture role <value>"
if step install hyperfine >/dev/null 2>&1; then echo "FAIL: installed before the question was answered" >&2; exit 1; fi
if step option set fixture role "Design" >/dev/null 2>&1; then echo "FAIL: a label was accepted as a value" >&2; exit 1; fi
step option set fixture role developer >/dev/null
expect "$(step list)" "★ Command-line benchmarking tool"           # recommended for everyone
expect "$(step list)" "★ A development tool"                         # recommended for developers
step option set fixture role design >/dev/null
if step list | grep -q "★ A development tool"; then echo "FAIL: still recommended for designers" >&2; exit 1; fi
step option set fixture role developer >/dev/null
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
# Missing tools and prerequisites (fake package manager and fake Homebrew).
if ! $real; then
    expect "$(step list)" "!  brew-only"
    expect "$(step list)" "needs Homebrew (brew)"
    if step install brew-only >/dev/null 2>&1; then echo "FAIL: installed although Homebrew is missing" >&2; exit 1; fi
    out=$(step install brewed)
    expect "$out" "brewed needs homebrew first"
    expect "$out" "Installed brewed"
    expect "$(step list)" "✓  brewed"
    expect "$(step list)" "·  brew-only"   # brew is there now
    step uninstall brewed >/dev/null && step uninstall homebrew >/dev/null
    expect "$(step list)" "!  brew-only"
fi

# Dependencies and updates (fake package manager only).
if ! $real; then
    plan=$(step install devtool --without toolbox --dry-run)
    expect "$plan" "First, dependency hyperfine"
    if grep -q "dependency toolbox" <<<"$plan"; then echo "FAIL: deselected dependency planned" >&2; exit 1; fi
    if step install devtool --without hyperfine >/dev/null 2>&1; then echo "FAIL: required dependency deselected" >&2; exit 1; fi
    out=$(step install devtool --without toolbox)
    expect "$out" "devtool needs hyperfine first"
    expect "$out" "Installed devtool"
    expect "$(step list)" "·  toolbox"
    # An update the package manager knows about makes the app "update available".
    echo 2.0 > "$tmp/pkgs/hyperfine.new"
    expect "$(step list)" "↑  hyperfine"
    expect "$(step install hyperfine --dry-run)" "Update hyperfine from 1.0 to 2.0"
    step settings --no-auto-update hyperfine >/dev/null
    expect "$(step upgrade)" "Not updated (automatic updates off): fixture/hyperfine"
    expect "$(step list)" "↑  hyperfine"
    step settings --auto-update hyperfine >/dev/null
    expect "$(step upgrade)" "Installed hyperfine"
    expect "$(step list)" "✓  hyperfine"
    [ "$(cat "$tmp/pkgs/hyperfine")" = 2.0 ] || { echo "FAIL: update not applied" >&2; exit 1; }
    step uninstall devtool >/dev/null && step uninstall hyperfine >/dev/null
fi

# Options (DSC parameters): collection value, app value, reinstall (fake package managers only).
if ! $real; then
    expect "$(step option list fixture)" "userInstall *false *default"
    expect "$(step install cli-tool)" "Installed cli-tool"
    [ -f "$tmp/pkgs/cli-tool" ] || { echo "FAIL: not installed system-wide" >&2; exit 1; }
    step option set fixture userInstall true >/dev/null
    expect "$(step option list cli-tool)" "userInstall *true *your value for the collection"
    expect "$(step list)" "installed with other options"
    expect "$(step list)" "✓  cli-tool"   # still checked with the values it was installed with
    out=$(step reinstall cli-tool)
    expect "$out" "cli-tool needs homebrew first"
    expect "$out" "Reinstalled cli-tool"
    [ ! -f "$tmp/pkgs/cli-tool" ] && [ -f "$tmp/brew-pkgs/cli-tool" ] || { echo "FAIL: not moved to a user install" >&2; exit 1; }
    step option set cli-tool userInstall false >/dev/null
    expect "$(step option list cli-tool)" "userInstall *false *your value for this app"
    step option reset cli-tool userInstall >/dev/null
    expect "$(step option list cli-tool)" "userInstall *true *your value for the collection"
    expect "$(step uninstall cli-tool)" "Uninstalled cli-tool"   # with the values it was installed with
    [ ! -f "$tmp/brew-pkgs/cli-tool" ] || { echo "FAIL: user install left behind" >&2; exit 1; }
    step option reset fixture userInstall >/dev/null
    step uninstall homebrew >/dev/null
fi

# A per-package-manager name: the fake pacman gets pacmanName (fake package managers only).
if ! $real; then
    expect "$(step install renamed)" "Installed renamed"
    [ -f "$tmp/pkgs/renamed" ] && [ ! -f "$tmp/pkgs/renamed-generic" ] || { echo "FAIL: pacmanName not used" >&2; exit 1; }
    expect "$(step list)" "✓  renamed"
    expect "$(step uninstall renamed)" "Uninstalled renamed"
fi

# Kits (fake package manager only: the fixture's packages don't exist for real).
if ! $real; then
    expect "$(step list)" "·  starter"
    plan=$(step install starter --without toolbox --with extras --dry-run)
    expect "$plan" "toolbox (not selected)"
    expect "$plan" "+ install        extras"
    if step install starter --without hyperfine >/dev/null 2>&1; then echo "FAIL: a required item was deselected" >&2; exit 1; fi
    expect "$(step install starter)" "Installed toolbox"   # default selection: hyperfine + toolbox
    expect "$(step list)" "◐  starter"                    # extras is not installed
    expect "$(step install starter --with extras)" "Installed extras"
    expect "$(step list)" "✓  starter"
    expect "$(step uninstall starter --only extras)" "Uninstalled extras"
    expect "$(step uninstall starter)" "Uninstalled hyperfine"
    expect "$(step list)" "·  starter"
fi

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
if $dsc; then
    expect "$(step list)" "·  dsc-file "
    expect "$(step install dsc-file)" "Installed dsc-file"
    [ "$(cat "$tmp/dsc-file.txt")" = furnish ] || { echo "FAIL: File/Content didn't write the file" >&2; exit 1; }
    expect "$(step list)" "✓  dsc-file "
    # DSC 3.3's File/Content can't delete (its manifest has no delete and doesn't handle _exist),
    # and the app has no removal steps: furnish refuses before changing anything.
    expect "$("$furnish" uninstall dsc-file --dry-run 2>&1 || true)" "furnish can't uninstall dsc-file"
    expect "$("$furnish" uninstall dsc-file 2>&1 || true)" "furnish can't uninstall dsc-file"
    [ -f "$tmp/dsc-file.txt" ] || { echo "FAIL: a refused uninstall changed something" >&2; exit 1; }
    expect "$(step list)" "✓  dsc-file "

    expect "$(step install dsc-removable)" "Installed dsc-removable"
    expect "$(step list)" "✓  dsc-removable "
    plan=$(step uninstall dsc-removable --dry-run)
    expect "$plan" "delete the file"
    grep -q "settings file" <<<"$plan" && { echo "FAIL: the preview shows the resources, not the removal steps" >&2; exit 1; }
    expect "$(step uninstall dsc-removable)" "Uninstalled dsc-removable"
    [ ! -e "$tmp/dsc-removable.txt" ] || { echo "FAIL: the removal steps left the file" >&2; exit 1; }
    expect "$(step list)" "·  dsc-removable "

    expect "$(step install dsc-stays)" "Installed dsc-stays"
    expect "$("$furnish" uninstall dsc-stays 2>&1 || true)" "dsc-stays is still installed"
    expect "$(step list)" "✓  dsc-stays "

    out=$("$furnish" install dsc-fails 2>&1) && { echo "FAIL: dsc-fails installed: $out" >&2; exit 1; }
    echo "$out" >&2
    expect "$out" "exited with code 1"
    [ ! -e "$tmp/dsc-fails.txt" ] || { echo "FAIL: furnish went on after the failed command" >&2; exit 1; }
    expect "$(step list)" "·  dsc-fails "

    out=$("$furnish" install dsc-differs 2>&1) && { echo "FAIL: dsc-differs installed: $out" >&2; exit 1; }
    echo "$out" >&2
    expect "$out" "dsc-differs isn't installed after the install: the resource 'settings file' (Microsoft.Filesystem.File/Content) still differs"
    grep -q "Installed dsc-differs" <<<"$out" && { echo "FAIL: furnish reported dsc-differs installed" >&2; exit 1; }
    expect "$(step list)" "·  dsc-differs "
fi

# Configuration resources, through DSC: install, test, drift, uninstall.
mkdir -p "$tmp/conf"
printf 'Host other.example\n  User me\n' > "$tmp/conf/ssh_config"
expect "$(step list)" "·  config "
plan=$(step install config --dry-run)
expect "$plan" "Add 'Include config.d/fixture.conf' to $tmp/conf/ssh_config"
expect "$plan" "Set \$.enabledPlugins\['fixture@market'\] in $tmp/conf/settings.json"
expect "$(step install config)" "Installed config"
expect "$(step list)" "✓  config "
[ "$(head -n 1 "$tmp/conf/ssh_config")" = "Include config.d/fixture.conf" ] || { echo "FAIL: the Include line isn't first" >&2; cat "$tmp/conf/ssh_config" >&2; exit 1; }
grep -q "^# BEGIN furnish fixture/config$" "$tmp/conf/ssh_config" || { echo "FAIL: no block" >&2; exit 1; }
# Git on Windows may check the collection out with CRLF line ends; the copy has the checkout's bytes.
[ "$(tr -d '\r' < "$tmp/conf/profile.conf")" = "$(cat "$tmp/repo/files/profile.conf")" ] || { echo "FAIL: profile not copied" >&2; exit 1; }
grep -q '"fixture@market": true' "$tmp/conf/settings.json" || { echo "FAIL: settings key missing" >&2; exit 1; }
[ "$(git config --file "$tmp/conf/gitconfig" fixture.server)" = https://fixture.example ] || { echo "FAIL: command didn't run" >&2; exit 1; }
[ -f ~/"$home_file" ] || { echo "FAIL: ~/ wasn't expanded" >&2; exit 1; }
case "$(uname -s)" in
MINGW* | MSYS*) ;;  # an access list instead of a mode
*) [ "$(stat -c %a "$tmp/conf/profile.conf" 2>/dev/null || stat -f %Lp "$tmp/conf/profile.conf")" = 600 ] || { echo "FAIL: profile isn't private" >&2; exit 1; } ;;
esac
# A value changed by hand shows as drift; installing again fixes it.
settings=$(cat "$tmp/conf/settings.json") && printf '%s\n' "${settings//true/false}" > "$tmp/conf/settings.json"  # no sed in nixos/nix
if step list | grep -q "✓  config "; then echo "FAIL: a changed value isn't drift" >&2; exit 1; fi
expect "$(step install config)" "Installed config"
expect "$(step list)" "✓  config "
expect "$(step uninstall config)" "Uninstalled config"
# Removing an outdated setting (its test fails) still runs remove. The app then counts as not
# installed, so this calls the resource the way DSC does.
git config --file "$tmp/conf/gitconfig" fixture.server https://old.fixture.example
printf '{"test": {"command": ["git", "config", "--file", "%s", "fixture.server"], "output": "https://fixture.example"}, "set": ["true"], "remove": ["git", "config", "--file", "%s", "--unset-all", "fixture.server"], "_exist": false}' \
    "$tmp/conf/gitconfig" "$tmp/conf/gitconfig" | "$furnish" dsc-resource command set >/dev/null
if [ -n "$(git config --file "$tmp/conf/gitconfig" fixture.server)" ]; then echo "FAIL: remove didn't run for an outdated setting" >&2; exit 1; fi
expect "$(step list)" "·  config "
[ "$(cat "$tmp/conf/ssh_config")" = "$(printf 'Host other.example\n  User me')" ] || { echo "FAIL: removal left lines behind" >&2; cat "$tmp/conf/ssh_config" >&2; exit 1; }
[ ! -e "$tmp/conf/profile.conf" ] && [ ! -e ~/"$home_file" ] || { echo "FAIL: files left behind" >&2; exit 1; }
ls "$tmp/conf"/ssh_config.*~ >/dev/null || { echo "FAIL: no backup of ssh_config" >&2; exit 1; }
[ -d "$tmp/conf" ] || { echo "FAIL: a folder that isn't empty was removed" >&2; exit 1; }

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
# Admin policy in the OS's native store (real mode; needs admin rights), else via the test override.
step collection remove lfs >/dev/null 2>&1 || true
cat > "$tmp/policy.json" <<JSON
{"settings": {"updateIntervalMinutes": {"value": 60, "locked": true}},
 "collections": {"preset": [{"name": "org", "url": "$tmp/repo"}], "usersCanAdd": false},
 "parameters": {"org": {"userInstall": {"value": true, "locked": true}, "role": {"value": "developer"}}}}
JSON
policy_id=io.github.furnish-self-service.furnish
if $real && [ "$(uname -s)" = Darwin ]; then
    plutil -convert xml1 -o "$tmp/policy.plist" "$tmp/policy.json"
    sudo mkdir -p "/Library/Managed Preferences" && sudo cp "$tmp/policy.plist" "/Library/Managed Preferences/$policy_id.plist"
    store="managed preferences"; drop() { sudo rm -f "/Library/Managed Preferences/$policy_id.plist"; }
elif $real && command -v powershell >/dev/null; then
    powershell -NoProfile -Command "New-Item -Path 'HKLM:\SOFTWARE\Policies\furnish' -Force | Out-Null; Set-ItemProperty -Path 'HKLM:\SOFTWARE\Policies\furnish' -Name Policy -Value (Get-Content -Raw '$tmp/policy.json')"
    store="registry"; drop() { powershell -NoProfile -Command "Remove-Item -Path 'HKLM:\SOFTWARE\Policies\furnish' -Recurse"; }
elif $real && [ "$(id -u)" = 0 ]; then
    mkdir -p /etc/furnish && cp "$tmp/policy.json" /etc/furnish/policy.json
    store="/etc/furnish"; drop() { rm -f /etc/furnish/policy.json; }
else
    export FURNISH_POLICY_FILE="$tmp/policy.json"
    store="FURNISH_POLICY_FILE"; drop() { unset FURNISH_POLICY_FILE; }
fi
expect "$(step collection list)" "managed by your organization"
expect "$(step settings)" "every 60 minutes  (set by your organization)"
if step settings --update-interval 5 >/dev/null 2>&1; then echo "FAIL: changed a locked setting" >&2; exit 1; fi
if step collection remove org >/dev/null 2>&1; then echo "FAIL: removed a managed collection" >&2; exit 1; fi
if step collection add "$tmp/repo" --name mine >/dev/null 2>&1; then echo "FAIL: added a collection against the policy" >&2; exit 1; fi
expect "$(step option list org/cli-tool)" "userInstall *true *your organization (locked)"
if "$furnish" list 2>&1 | grep -q "org asks"; then echo "FAIL: asked although the policy answers" >&2; exit 1; fi
if step option set org/cli-tool userInstall false >/dev/null 2>&1; then echo "FAIL: changed a locked option" >&2; exit 1; fi
drop
echo "e2e OK ($lfs, $n installer(s), policy from $store, DSC resources $($dsc && echo used || echo "not beside furnish"))" >&2
echo "e2e OK ($($real && echo real || echo fake) package manager)"
