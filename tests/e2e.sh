#!/bin/sh
# End-to-end check: add a collection, install, update detection, uninstall.
# Uses a fake pacman (and pkexec/sudo) so it needs no root and touches no real packages.
# Usage: tests/e2e.sh [path/to/furnish]   (default: target/fast/furnish)
set -eu
furnish=$(realpath "${1:-target/fast/furnish}")
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export FURNISH_DATA_DIR="$tmp/data"

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

# Collection with one app.
mkdir -p "$tmp/repo/apps"
cat > "$tmp/repo/apps/demo.dsc.yaml" <<'YAML'
metadata:
  furnish:
    description: Demo app
    categories: [Test]
resources:
  - name: demo package
    type: Furnish/Package
    properties:
      name: demo
YAML
git -C "$tmp/repo" init -q
git -C "$tmp/repo" add -A
git -C "$tmp/repo" -c user.name=t -c user.email=t@t commit -qm init

check() { echo "+ furnish $*" >&2; "$furnish" "$@"; }
expect() { echo "$1" | grep -q "$2" || { echo "FAIL: expected '$2' in:" >&2; echo "$1" >&2; exit 1; }; }

check collection add "$tmp/repo" --name demo-collection >/dev/null
expect "$(check list)" "·  demo"
expect "$(check install demo)" "Installed demo"
expect "$(check list)" "✓  demo"
expect "$(check install demo-collection/demo)" "already installed"
expect "$(check uninstall demo)" "Uninstalled demo"
expect "$(check list)" "·  demo"
check collection remove demo-collection >/dev/null
echo "e2e OK"
