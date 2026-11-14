#!/bin/sh
# The furnish launchd daemon on macOS: a standard user in the allowed group installs a .pkg the
# organization provides; other users can't; users can't change the service's collections.
# Needs passwordless sudo on a throwaway machine (CI): creates users and a daemon.
#
#   tests/service-macos.sh target/release/furnish
set -eu
binary=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
repo=$(cd "$(dirname "$0")/.." && pwd)
label=io.github.furnish-self-service.furnish
fail() { echo "FAIL: $*" >&2; sudo cat /var/log/furnish.log >&2 || true; exit 1; }
as() { user=$1; shift; sudo -u "$user" env FURNISH_DATA_DIR="/tmp/$user-data" HOME=/tmp "$@"; }
installed() { pkgutil --pkg-info io.github.furnish-self-service.demo >/dev/null 2>&1; }

for u in alice bob; do sudo sysadminctl -addUser "$u" -password Furnish-Test-1 >/dev/null 2>&1; done
sudo dseditgroup -o create furnish-users
sudo dseditgroup -o edit -a alice -t user furnish-users
sudo install -m 755 "$binary" /usr/local/bin/furnish

# The organization's collection: one app, a .pkg.
work=$(cd "$(mktemp -d /tmp/furnish-service.XXXXXX)" && pwd -P); chmod 755 "$work"  # /tmp is a symlink on macOS
mkdir -p "$work/org/apps" "$work/org/installers" "$work/pkgroot/opt/furnish-demo"
echo demo > "$work/pkgroot/opt/furnish-demo/demo.txt"
pkgbuild --quiet --root "$work/pkgroot" --identifier io.github.furnish-self-service.demo --version 1.0 \
    --install-location / "$work/org/installers/furnish-demo.pkg"
cat > "$work/org/apps/demo.dsc.yaml" <<YAML
metadata:
  furnish:
    description: Installer test
resources:
  - name: demo installer
    type: Furnish/Installer
    properties:
      file: installers/furnish-demo.pkg
      packageId: io.github.furnish-self-service.demo
YAML
git -C "$work/org" init -q && git -C "$work/org" add -A && git -C "$work/org" -c user.name=t -c user.email=t@t commit -qm apps
sudo chown -R root "$work/org"  # root (the service) only clones repositories it owns or trusts

# The policy, as an MDM profile would deploy it.
printf '{"collections": {"preset": [{"name": "org", "url": "%s"}], "usersCanAdd": false}, "service": {"allowedGroup": "furnish-users"}}' \
    "$work/org" > "$work/policy.json"
sudo mkdir -p "/Library/Managed Preferences"
plutil -convert xml1 -o - "$work/policy.json" | sudo tee "/Library/Managed Preferences/$label.plist" >/dev/null

sudo install -m 644 "$repo/packaging/macos/$label.plist" /Library/LaunchDaemons/
sudo launchctl bootstrap system "/Library/LaunchDaemons/$label.plist"
for _ in $(seq 100); do [ -S /var/run/furnish.sock ] && [ -d "/Library/Application Support/furnish/collections/org/apps" ] && break; sleep 0.3; done
[ -S /var/run/furnish.sock ] || fail "the service didn't start"

echo "== a member of the allowed group installs system-wide"
as alice furnish install demo || fail "alice couldn't install"
installed || fail "not installed"

echo "== others are refused"
if out=$(as bob furnish uninstall demo 2>&1); then fail "bob uninstalled"; fi
echo "$out" | grep -q "aren't allowed" || fail "unexpected refusal: $out"
installed || fail "removed anyway"

echo "== users can't change what the service installs"
as alice sh -c 'echo x > "/Library/Application Support/furnish/collections/org/apps/evil.dsc.yaml"' 2>/dev/null || true
[ ! -e "/Library/Application Support/furnish/collections/org/apps/evil.dsc.yaml" ] || fail "alice wrote into the collection"

echo "== uninstall through the service"
as alice furnish uninstall demo || fail "alice couldn't uninstall"
if installed; then fail "still installed"; fi

sudo launchctl bootout "system/$label"
echo "service OK"
