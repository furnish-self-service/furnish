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
step collection remove fixture >/dev/null
echo "e2e OK ($($real && echo real || echo fake) package manager)"
