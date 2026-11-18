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
  # The sync database's version: "<name>.new" (tests set it), else the installed one, else 1.0.
  -Si) v=1.0; for f in "$tmp/pkgs/\$2" "$tmp/pkgs/\$2.new"; do [ -f "\$f" ] && v=\$(cat "\$f"); done
       printf 'Repository      : extra\nName            : %s\nVersion         : %s\n' "\$2" "\$v" ;;
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

# Required versions (properties.versions). No package version meets "unmet" (hyperfine >= 999),
# on every system. "required" (fake pacman) needs 2.0, which tests offer later. "pinned" (real
# apt, dnf, zypper) needs an older version than the newest one on offer, if there is one.
cat > "$tmp/repo/apps/unmet.dsc.yaml" <<'YAML'
metadata:
  furnish:
    description: A version nobody offers
    categories: [Test]
resources:
  - name: hyperfine (winget)
    type: Furnish/Package
    condition: "[equals(context().os.family, 'Windows')]"
    properties: { name: sharkdp.hyperfine, versions: [">=999"] }
  - name: hyperfine
    type: Furnish/Package
    condition: "[not(equals(context().os.family, 'Windows'))]"
    properties: { name: hyperfine, versions: [">=999"] }
YAML
if ! $real; then
    cat > "$tmp/repo/apps/required.dsc.yaml" <<'YAML'
metadata:
  furnish:
    description: Needs version 2.0 or later
    categories: [Test]
resources:
  - name: required-tool
    type: Furnish/Package
    condition: "[not(equals(context().os.family, 'Windows'))]"
    properties: { name: required-tool, versions: [">=2.0"] }
YAML
fi
# Pinning: a package with two versions on offer, asked from the resource itself.
pin_pkg= pin_version=
if $real && [ "$(id -u)" = 0 ] && { command -v apt-get || command -v dnf || command -v zypper; } >/dev/null 2>&1; then
    res="$(dirname "$0")/../resources/unix/furnish-package.dsc.resource.sh"
    for p in tzdata hyperfine; do
        state=$(name=$p reportVersions=all sh "$res" get 2>/dev/null) || continue
        latest=$(printf '%s' "$state" | sed -n 's/.*"latestVersion":"\([^"]*\)".*/\1/p')
        for v in $(printf '%s' "$state" | sed -n 's/.*"availableVersions":\[\([^]]*\)\].*/\1/p' | tr -d '"' | tr ',' ' '); do
            [ "$v" != "$latest" ] && { pin_pkg=$p pin_version=$v; break 2; }
        done
    done
    if [ -n "$pin_pkg" ]; then
        cat > "$tmp/repo/apps/pinned.dsc.yaml" <<YAML
metadata:
  furnish:
    description: An older version than the newest
    categories: [Test]
resources:
  - name: $pin_pkg
    type: Furnish/Package
    condition: "[equals(context().os.family, 'Linux')]"
    properties: { name: $pin_pkg, versions: ["$pin_version"] }
YAML
    fi
fi

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
trap 'rm -rf "$tmp" ~/"$home_file".gitconfig ~/"$home_file".ovpn ~/"$home_file".imported' EXIT
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
    properties: { path: "$tmp/conf/settings.json", location: "\$.enabledPlugins['fixture@market']", value: true, private: true }
  - name: home file
    type: Furnish/File
    properties: { path: "~/$home_file", content: "furnish" }
  - name: current context
    type: Furnish/Value
    properties: { path: "$tmp/conf/kube.yaml", location: "\$.current-context", removeIf: [fixture-prod, fixture-stg] }
  - name: server
    type: Furnish/Command
    properties:
      test: { command: [git, config, --file, "$tmp/conf/gitconfig", fixture.server], output: https://fixture.example }
      # There, but outdated: any server set (an empty value is none).
      present: { command: [git, config, --file, "$tmp/conf/gitconfig", --get-regexp, '^fixture\.server$', .] }
      set: [git, config, --file, "$tmp/conf/gitconfig", fixture.server, https://fixture.example]
      remove: [git, config, --file, "$tmp/conf/gitconfig", fixture.server, ""]  # succeeds also with nothing to remove
  - name: marketplace
    type: Furnish/Command
    properties:
      test: { command: [git, config, --file, "$tmp/conf/gitconfig", fixture.marketplace], output: acme }
      set: [git, config, --file, "$tmp/conf/gitconfig", fixture.marketplace, acme]
      remove: keep  # other apps use it too
  - name: home setting
    type: Furnish/Command
    properties:
      test: { command: [git, config, --file, "~/$home_file.gitconfig", fixture.home], output: home }
      set: [git, config, --file, "~/$home_file.gitconfig", fixture.home, home]
      remove: [git, config, --file, "~/$home_file.gitconfig", fixture.home, ""]
  # A tool's own copy of a file the app writes (cat and cp stand in for the tool; on Windows,
  # Git's): the output is compared with the file, without its comment and blank lines.
  - name: vpn file
    type: Furnish/File
    properties: { path: "~/$home_file.ovpn", content: "# Fixture VPN\nremote vpn.fixture.example 1194\n", private: true }
  - name: vpn import
    type: Furnish/Command
    properties:
      test: { command: [cat, "~/$home_file.imported"], file: "~/$home_file.ovpn", ignoreLines: '^(#|$)' }
      present: { command: [cat, "~/$home_file.imported"] }
      set: [cp, "~/$home_file.ovpn", "~/$home_file.imported"]
      remove: [rm, -f, "~/$home_file.imported"]
YAML

# A profile import with only a temporary input (Git Bash supplies cat, cp and rm on Windows).
cat > "$tmp/repo/apps/command-input.dsc.yaml" <<YAML
resources:
  - name: import
    type: Furnish/Command
    properties:
      source: files/profile.conf
      test: { command: [cat, "$tmp/tool-copy"], file: '%s' }
      set: [cp, '%s', "$tmp/tool-copy"]
      remove: [rm, -f, "$tmp/tool-copy"]
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
    [ -f "$tmp/brew-pkgs/brewed-tool" ] && [ ! -f "$tmp/brew-pkgs/brewed-generic" ] || { echo "FAIL: brewName not used" >&2; exit 1; }
    expect "$(step list)" "✓  brewed"
    expect "$(step list)" "·  brew-only"   # brew is there now
    step uninstall brewed >/dev/null && step uninstall homebrew >/dev/null
    expect "$(step list)" "!  brew-only"
fi

# A package's program installed without furnish (Furnish/Package's command), e.g. with npm or the
# vendor's installer: installed, but furnish neither installs it again nor removes it.
expect "$(step list)" "·  elsewhere "
mkdir -p "$tmp/elsewhere"
if command -v cygpath >/dev/null; then
    printf '@echo off\r\n' > "$tmp/elsewhere/elsewhere-tool.cmd"
else
    printf '#!/bin/sh\n' > "$tmp/elsewhere/elsewhere-tool" && chmod +x "$tmp/elsewhere/elsewhere-tool"
fi
elsewhere=$tmp/elsewhere
if command -v cygpath >/dev/null; then elsewhere=$(cygpath -u "$elsewhere"); fi  # PATH separates with ':'
PATH="$elsewhere:$PATH"
list=$(step list)
expect "$list" "✓  elsewhere "
expect "$list" "installed outside furnish: .*elsewhere-tool"
expect "$list" "version unknown: can't check the required versions"
expect "$(step install elsewhere)" "elsewhere is already installed: it was installed outside furnish"
if ! $real; then [ ! -f "$tmp/pkgs/elsewhere-tool" ] || { echo "FAIL: installed a second copy" >&2; exit 1; }; fi
out=$("$furnish" uninstall elsewhere 2>&1) && { echo "FAIL: uninstalled a program furnish didn't install: $out" >&2; exit 1; }
expect "$out" "elsewhere was installed outside furnish (.*elsewhere-tool.*): uninstall it the way it was installed"
expect "$(step list)" "✓  elsewhere "
rm -f "$tmp"/elsewhere/elsewhere-tool*
expect "$(step list)" "·  elsewhere "
if ! $real; then
    # Without Homebrew, the program alone makes it installed; without both it needs Homebrew.
    expect "$(step list)" "!  brew-elsewhere "
    printf '#!/bin/sh\n' > "$tmp/elsewhere/brew-elsewhere-tool" && chmod +x "$tmp/elsewhere/brew-elsewhere-tool"
    expect "$(step list)" "✓  brew-elsewhere "
    rm -f "$tmp/elsewhere/brew-elsewhere-tool"
    # Not only on PATH: in the folders installers use, e.g. ~/.local/bin, which an app started
    # from the Finder doesn't have on its PATH.
    mkdir -p "$tmp/home/.local/bin"
    printf '#!/bin/sh\n' > "$tmp/home/.local/bin/elsewhere-tool" && chmod +x "$tmp/home/.local/bin/elsewhere-tool"
    out=$(HOME="$tmp/home" name=elsewhere-tool command=elsewhere-tool sh "$FURNISH_DATA_DIR/resources/furnish-package.dsc.resource.sh" get)
    expect "$out" "\"_exist\":true,\"managedBy\":\"other\",\"path\":\"$tmp/home/.local/bin/elsewhere-tool\""
    out=$(HOME="$tmp/home" name=elsewhere-tool command=elsewhere-tool sh "$FURNISH_DATA_DIR/resources/furnish-package.dsc.resource.sh" test)
    expect "$out" '"_inDesiredState":true'
    # Not something else that happens to carry the name.
    rm "$tmp/home/.local/bin/elsewhere-tool" && mkdir "$tmp/home/.local/bin/elsewhere-tool"
    out=$(HOME="$tmp/home" name=elsewhere-tool command=elsewhere-tool sh "$FURNISH_DATA_DIR/resources/furnish-package.dsc.resource.sh" get)
    expect "$out" '"_exist":false'
    if (name=x command=../x sh "$FURNISH_DATA_DIR/resources/furnish-package.dsc.resource.sh" get) >/dev/null 2>&1; then
        echo "FAIL: a command with a path was accepted" >&2; exit 1
    fi
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

# Required versions. The expected texts, in one place.
needs_unmet="needs 999 or later"
if ! $real; then
    # Nothing acceptable on offer: "!" and why; install fails, the preview says why.
    expect "$(step list)" "!  unmet"
    expect "$(step list)" "$needs_unmet; pacman offers 1.0"
    out=$("$furnish" install unmet 2>&1) && { echo "FAIL: unmet installed: $out" >&2; exit 1; }
    echo "$out" >&2
    expect "$out" "$needs_unmet; pacman offers 1.0"
    [ ! -f "$tmp/pkgs/hyperfine" ] || { echo "FAIL: unmet installed something" >&2; exit 1; }
    expect "$("$furnish" install unmet --dry-run 2>&1 || true)" "No acceptable version: $needs_unmet"

    # Installed 1.0, needs 2.0, nothing newer yet: installed but needs a version nobody offers.
    expect "$(step install required --dry-run 2>&1 || true)" "No acceptable version: needs 2.0 or later; pacman offers 1.0"
    echo 1.0 > "$tmp/pkgs/required-tool"
    expect "$(step list)" "(needs 2.0 or later; pacman offers 1.0)"
    # 2.0 on offer: a required update, installed by upgrade even with automatic updates off.
    echo 2.0 > "$tmp/pkgs/required-tool.new"
    expect "$(step list)" "↑  required"
    expect "$(step list)" "(security update required: 2.0 or later; pacman offers 2.0)"
    expect "$(step install required --dry-run)" "Update required-tool from 1.0 to 2.0 with pacman"
    step settings --no-auto-update required >/dev/null
    expect "$(step upgrade)" "Installed required"
    [ "$(cat "$tmp/pkgs/required-tool")" = 2.0 ] || { echo "FAIL: required update not applied" >&2; exit 1; }
    expect "$(step list)" "✓  required"
    step settings --auto-update required >/dev/null
    expect "$(step uninstall required)" "Uninstalled required"
else
    # Real package managers: hyperfine >= 999 exists nowhere; the error names what is offered.
    out=$("$furnish" install unmet 2>&1) && { echo "FAIL: unmet installed: $out" >&2; exit 1; }
    echo "$out" >&2
    expect "$out" "$needs_unmet"
    expect "$out" "offers"
    if [ -n "$pin_pkg" ]; then
        echo "--- pinning $pin_pkg $pin_version" >&2
        expect "$(step install pinned --dry-run)" "$pin_version"
        step install pinned >/dev/null
        got=$(name=$pin_pkg sh "$res" get 2>/dev/null | sed -n 's/.*"version":"\([^"]*\)".*/\1/p')
        [ "$got" = "$pin_version" ] || { echo "FAIL: $pin_pkg is $got, not $pin_version" >&2; exit 1; }
        expect "$(step list)" "✓  pinned"
    elif [ "$(id -u)" = 0 ]; then
        echo "(pinning skipped: no apt, dnf or zypper, or no package with two versions on offer)" >&2
    fi
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
printf 'current-context: fixture-prod\n' > "$tmp/conf/kube.yaml"
expect "$(step list)" "·  config "
# A matching hand import predates the file Furnish/File writes. An import would overwrite
# its comment, so the marker verifies that the set command never ran.
printf '# imported by hand\nremote vpn.fixture.example 1194\n' > ~/"$home_file".imported
plan=$(step --verbose install config --dry-run 2>&1)
if grep -Fq 'remote vpn.fixture.example 1194' <<<"$plan"; then echo "FAIL: a private profile appeared in the preview or its log" >&2; exit 1; fi
expect "$plan" "Add 'Include config.d/fixture.conf' to $tmp/conf/ssh_config"
expect "$plan" "Run: git config --file ~/$home_file.gitconfig fixture.home home"
expect "$plan" "Set \$.enabledPlugins\['fixture@market'\] in $tmp/conf/settings.json"
expect "$plan" "= nothing to do  vpn import"
[ ! -e ~/"$home_file".ovpn ] || { echo "FAIL: preview wrote the profile" >&2; exit 1; }
out=$(step install config)
expect "$out" "Installed config"
expect "$out" "vpn import already as described"
grep -q '^# imported by hand$' ~/"$home_file".imported || { echo "FAIL: a matching profile was reimported" >&2; exit 1; }
# A stale local file is also replaced before comparing, and is untouched by the dry run.
printf 'remote stale.example 1194\n' > ~/"$home_file".ovpn
expect "$(step install config --dry-run)" "= nothing to do  vpn import"
grep -q '^remote stale.example 1194$' ~/"$home_file".ovpn || { echo "FAIL: preview changed the stale profile" >&2; exit 1; }
expect "$(step install config)" "vpn import already as described"
grep -q '^# imported by hand$' ~/"$home_file".imported || { echo "FAIL: fixing the file replaced the imported profile" >&2; exit 1; }
expect "$(step list)" "✓  config "
[ "$(head -n 1 "$tmp/conf/ssh_config")" = "Include config.d/fixture.conf" ] || { echo "FAIL: the Include line isn't first" >&2; cat "$tmp/conf/ssh_config" >&2; exit 1; }
grep -q "^# BEGIN furnish fixture/config$" "$tmp/conf/ssh_config" || { echo "FAIL: no block" >&2; exit 1; }
# Git on Windows may check the collection out with CRLF line ends; the copy has the checkout's bytes.
[ "$(tr -d '\r' < "$tmp/conf/profile.conf")" = "$(cat "$tmp/repo/files/profile.conf")" ] || { echo "FAIL: profile not copied" >&2; exit 1; }
grep -q '"fixture@market": true' "$tmp/conf/settings.json" || { echo "FAIL: settings key missing" >&2; exit 1; }
[ "$(git config --file "$tmp/conf/gitconfig" fixture.server)" = https://fixture.example ] || { echo "FAIL: command didn't run" >&2; exit 1; }
[ -f ~/"$home_file" ] || { echo "FAIL: ~/ wasn't expanded" >&2; exit 1; }
[ "$(git config --file ~/"$home_file".gitconfig fixture.home)" = home ] || { echo "FAIL: ~/ in a command wasn't expanded" >&2; exit 1; }
# removeIf: installing leaves the value as it is, there or not, and the app stays installed.
grep -q '^current-context: fixture-prod$' "$tmp/conf/kube.yaml" || { echo "FAIL: install changed current-context" >&2; exit 1; }
printf 'kind: Config\n' > "$tmp/conf/kube.yaml"
expect "$(step list)" "✓  config "
printf 'current-context: fixture-prod\n' > "$tmp/conf/kube.yaml"
expect "$(step list)" "✓  config "
# private: a copied file, and a file that a Furnish/Value step creates.
for f in profile.conf settings.json; do
    case "$(uname -s)" in
    # An access list for the user only, nothing inherited.
    MINGW* | MSYS*) acl=$(MSYS_NO_PATHCONV=1 icacls "$tmp/conf/$f") && echo "$acl" >&2
                    ! grep -q '(I)' <<<"$acl" || { echo "FAIL: $f inherits access" >&2; exit 1; } ;;
    *) [ "$(stat -c %a "$tmp/conf/$f" 2>/dev/null || stat -f %Lp "$tmp/conf/$f")" = 600 ] || { echo "FAIL: $f isn't private" >&2; exit 1; } ;;
    esac
done
# A value changed by hand shows as drift; installing again fixes it.
settings=$(cat "$tmp/conf/settings.json") && printf '%s\n' "${settings//true/false}" > "$tmp/conf/settings.json"  # no sed in nixos/nix
if step list | grep -q "✓  config "; then echo "FAIL: a changed value isn't drift" >&2; exit 1; fi
expect "$(step install config)" "Installed config"
expect "$(step list)" "✓  config "
# The tool's copy without comment lines, with blank lines and Windows line ends: still the same.
printf 'remote vpn.fixture.example 1194\r\n\r\n' > ~/"$home_file".imported
expect "$(step list)" "✓  config "
printf 'remote vpn.other.example 1194\r\n' > ~/"$home_file".imported
expect "$(step install config --dry-run)" "Test: the output of cat ~/$home_file.imported differs from ~/$home_file.ovpn"
expect "$(step install config)" "Installed config"
expect "$(step list)" "✓  config "
# An outdated setting (test fails, present passes) is an update, and uninstalling removes it.
git config --file "$tmp/conf/gitconfig" fixture.server https://old.fixture.example
expect "$(step list)" "↑  config "
expect "$(step install config --dry-run)" "finds the setting: it is there but outdated"
plan=$(step uninstall config --dry-run)
expect "$plan" "Remove \$.current-context from $tmp/conf/kube.yaml"
expect "$plan" "Keep what git config --file $tmp/conf/gitconfig fixture.marketplace acme set up"
expect "$(step uninstall config)" "Uninstalled config"
if grep -q current-context "$tmp/conf/kube.yaml"; then echo "FAIL: uninstall left the app's current-context" >&2; exit 1; fi
# A file that a Furnish/Value step edits is the user's: uninstall keeps it, private.
[ -f "$tmp/conf/settings.json" ] || { echo "FAIL: uninstall deleted settings.json" >&2; exit 1; }
case "$(uname -s)" in
MINGW* | MSYS*) ;;
*) [ "$(stat -c %a "$tmp/conf/settings.json" 2>/dev/null || stat -f %Lp "$tmp/conf/settings.json")" = 600 ] || { echo "FAIL: uninstall changed the mode of settings.json" >&2; exit 1; } ;;
esac
[ "$(git config --file "$tmp/conf/gitconfig" fixture.marketplace)" = acme ] || { echo "FAIL: remove: keep didn't keep the setting" >&2; exit 1; }
[ -z "$(git config --file ~/"$home_file".gitconfig fixture.home)" ] || { echo "FAIL: remove with ~/ didn't run" >&2; exit 1; }
[ ! -e ~/"$home_file".imported ] && [ ! -e ~/"$home_file".ovpn ] || { echo "FAIL: the VPN files were left behind" >&2; exit 1; }
if [ -n "$(git config --file "$tmp/conf/gitconfig" fixture.server)" ]; then echo "FAIL: remove didn't run for an outdated setting" >&2; exit 1; fi
expect "$(step list)" "·  config "
[ "$(cat "$tmp/conf/ssh_config")" = "$(printf 'Host other.example\n  User me')" ] || { echo "FAIL: removal left lines behind" >&2; cat "$tmp/conf/ssh_config" >&2; exit 1; }
[ ! -e "$tmp/conf/profile.conf" ] && [ ! -e ~/"$home_file" ] || { echo "FAIL: files left behind" >&2; exit 1; }
ls "$tmp/conf"/ssh_config.*~ >/dev/null || { echo "FAIL: no backup of ssh_config" >&2; exit 1; }
[ -d "$tmp/conf" ] || { echo "FAIL: a folder that isn't empty was removed" >&2; exit 1; }

# A matching copy previously imported by the tool is sufficient on its own.
cp "$tmp/repo/files/profile.conf" "$tmp/tool-copy"
expect "$(step install command-input)" "already installed"
printf 'outdated\n' > "$tmp/tool-copy"
plan=$(step install command-input --dry-run)
expect "$plan" "differs from profile.conf"
if [[ "$plan" == *vpn.fixture.example* ]]; then echo "FAIL: private command input in preview" >&2; exit 1; fi
expect "$(step install command-input)" "Installed command-input"
git diff --no-index --quiet -- "$tmp/repo/files/profile.conf" "$tmp/tool-copy"
expect "$(step uninstall command-input)" "Uninstalled command-input"
[ ! -e "$tmp/tool-copy" ] || { echo "FAIL: profile still imported after uninstall" >&2; exit 1; }

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
