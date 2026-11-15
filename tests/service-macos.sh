#!/bin/sh
# The furnish launchd daemon on macOS: a standard user in the allowed group installs a .pkg the
# organization provides; other users can't; users can't change the service's collections. The
# service runs Homebrew as its owner (the runner account) with furnish's sudo rule, and refuses
# Homebrew that others can change.
# Needs passwordless sudo on a throwaway machine (CI): creates users and a daemon, changes sudo's
# configuration and /opt/homebrew.
#
#   tests/service-macos.sh target/release/furnish
set -eu
binary=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
label=io.github.furnish-self-service.furnish
fail() { echo "FAIL: $*" >&2; sudo cat /var/log/furnish.log >&2 || true; exit 1; }
as() { user=$1; shift; sudo -u "$user" env FURNISH_DATA_DIR="/tmp/$user-data" HOME=/tmp "$@"; }
# The service runs only a binary that root owns, in folders only root (or admins) can change.
furnish=/Library/furnish-test/furnish
installed() { pkgutil --pkg-info io.github.furnish-self-service.demo >/dev/null 2>&1; }

for u in alice bob; do sudo sysadminctl -addUser "$u" -password Furnish-Test-1 >/dev/null 2>&1; done
sudo dseditgroup -o create furnish-users
sudo dseditgroup -o edit -a alice -t user furnish-users
sudo mkdir -p /Library/furnish-test
sudo install -o root -g wheel -m 755 "$binary" "$furnish"

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
# A cask with an installer package, from a local tap: brew runs `sudo installer` for it.
mkdir -p "$work/brewroot/opt/furnish-brewdemo"
echo brewdemo > "$work/brewroot/opt/furnish-brewdemo/demo.txt"
pkgbuild --quiet --root "$work/brewroot" --identifier io.github.furnish-self-service.brewdemo --version 1.0 \
    --install-location / "$work/furnish-brewdemo.pkg"
chmod 644 "$work/furnish-brewdemo.pkg"
brew_owner=$(stat -f %Su "$(brew --repository)")
brew tap-new --no-git furnish/test >/dev/null
mkdir -p "$(brew --repository)/Library/Taps/furnish/homebrew-test/Casks"
cat > "$(brew --repository)/Library/Taps/furnish/homebrew-test/Casks/furnish-brewdemo.rb" <<RUBY
cask "furnish-brewdemo" do
  version "1.0"
  sha256 :no_check
  url "file://$work/furnish-brewdemo.pkg"
  name "furnish brew demo"
  desc "Test package for furnish"
  homepage "https://github.com/furnish-self-service/furnish"
  pkg "furnish-brewdemo.pkg"
  uninstall pkgutil: "io.github.furnish-self-service.brewdemo"
end
RUBY
cat > "$work/org/apps/brewdemo.dsc.yaml" <<YAML
metadata:
  furnish:
    description: Homebrew cask test
resources:
  - name: brew demo
    type: Furnish/Package
    properties:
      name: furnish-brewdemo
      packageManager: brew
YAML
git -C "$work/org" init -q && git -C "$work/org" add -A && git -C "$work/org" -c user.name=t -c user.email=t@t commit -qm apps
sudo chown -R root "$work/org"  # root (the service) only clones repositories it owns or trusts

# The policy, as an MDM profile would deploy it.
printf '{"collections": {"preset": [{"name": "org", "url": "%s"}], "usersCanAdd": false}, "service": {"allowedGroup": "furnish-users"}}' \
    "$work/org" > "$work/policy.json"
sudo mkdir -p "/Library/Managed Preferences"
plutil -convert xml1 -o - "$work/policy.json" | sudo tee "/Library/Managed Preferences/$label.plist" >/dev/null

echo "== the service doesn't run a binary that users can change"
cp "$binary" "$work/furnish-user"
if out=$(sudo "$work/furnish-user" service install 2>&1); then fail "installed a service binary that the runner user can change"; fi
echo "$out" | grep -q "not root" || fail "unexpected refusal: $out"
[ ! -e "/Library/LaunchDaemons/$label.plist" ] || fail "launchd definition written for it"

sudo "$furnish" service install || fail "service install failed"
[ -f "/Library/LaunchDaemons/$label.plist" ] || fail "no launchd definition"
grep -q "<string>/usr/bin:/bin:/usr/sbin:/sbin</string>" "/Library/LaunchDaemons/$label.plist" || fail "the service's PATH has folders users may own"
[ "$(stat -f %Lp /var/log/furnish.log)" = 600 ] || fail "users can read the service's log"
for _ in $(seq 100); do [ -S /var/run/furnish.sock ] && [ -d "/Library/Application Support/furnish/collections/org/apps" ] && break; sleep 0.3; done
[ -S /var/run/furnish.sock ] || fail "the service didn't start"

echo "== a member of the allowed group installs system-wide"
as alice "$furnish" install demo || fail "alice couldn't install"
installed || fail "not installed"

echo "== others are refused"
if out=$(as bob "$furnish" uninstall demo 2>&1); then fail "bob uninstalled"; fi
echo "$out" | grep -q "aren't allowed" || fail "unexpected refusal: $out"
installed || fail "removed anyway"

echo "== users can't change what the service installs"
as alice sh -c 'echo x > "/Library/Application Support/furnish/collections/org/apps/evil.dsc.yaml"' 2>/dev/null || true
[ ! -e "/Library/Application Support/furnish/collections/org/apps/evil.dsc.yaml" ] || fail "alice wrote into the collection"

echo "== uninstall through the service"
as alice "$furnish" uninstall demo || fail "alice couldn't uninstall"
if installed; then fail "still installed"; fi

echo "== Homebrew as its owner, with furnish's sudo rule"
brew_installed() { pkgutil --pkg-info io.github.furnish-self-service.brewdemo >/dev/null 2>&1; }
rule=/etc/sudoers.d/furnish-homebrew
[ ! -e "$rule" ] || fail "service install without --homebrew-sudo wrote $rule"
# The rule is refused while Homebrew belongs to someone who isn't an administrator.
sudo chown bob "$(brew --repository)"
if out=$(sudo "$furnish" service install --homebrew-sudo 2>&1); then fail "rule for a non-admin owner: $out"; fi
sudo chown "$brew_owner" "$(brew --repository)"
echo "$out" | grep -q "bob, who isn't an administrator" || fail "unexpected refusal: $out"
[ ! -e "$rule" ] || fail "rule written anyway"
# The runner's own passwordless sudo goes away, so brew's sudo can only work through furnish's
# rule. In one root shell, since the runner can't use sudo in between.
own=$(sudo grep -lE "^[[:space:]]*$brew_owner[[:space:]]" /etc/sudoers.d/* 2>/dev/null | grep -v "$rule" || true)
[ -n "$own" ] || { sudo grep -n "$brew_owner\|NOPASSWD" /etc/sudoers; sudo ls -la /etc/sudoers.d; fail "where does $brew_owner's sudo rule live?"; }
restore() { for f in $own; do sudo mv "$f.off" "$f"; done; }
sudo sh -c '
    owner=$1 furnish=$2; shift 2
    for f do mv "$f" "$f.off"; done
    sudo -u "$owner" sudo -K  # sudo also accepts a recent use instead of a password
    if sudo -u "$owner" sudo -n true 2>/dev/null; then
        echo "$owner can still use sudo:" >&2; sudo -l -U "$owner" >&2; grep -v "^#" /etc/sudoers | grep . >&2
        for f do mv "$f.off" "$f"; done; exit 1
    fi
    "$furnish" service install --homebrew-sudo
' sh "$brew_owner" "$furnish" $own || { sudo sh -c 'for f do mv "$f.off" "$f"; done' sh $own; fail "service install --homebrew-sudo failed"; }
trap restore EXIT
[ "$(stat -f '%Su:%Sg %Lp' "$rule")" = "root:wheel 440" ] || fail "$rule has owner and mode $(stat -f '%Su:%Sg %Lp' "$rule")"
sudo grep -qx "$brew_owner ALL = (root) NOPASSWD: ALL" "$rule" || fail "unexpected rule: $(sudo cat "$rule")"
sudo -u "$brew_owner" sudo -n true || fail "the rule doesn't work"
for _ in $(seq 100); do [ -S /var/run/furnish.sock ] && break; sleep 0.3; done
as alice "$furnish" -v install brewdemo || fail "alice couldn't install the cask"
brew_installed || fail "the cask's package isn't installed"
[ "$(stat -f %Su "$(brew --prefix)/Caskroom/furnish-brewdemo")" = "$brew_owner" ] || fail "brew didn't run as $brew_owner"
as alice "$furnish" list | grep -q "✓  brewdemo " || fail "alice doesn't see it installed"
as alice "$furnish" -v uninstall brewdemo || fail "alice couldn't uninstall the cask"
if brew_installed; then fail "the cask's package is still installed"; fi
restore
trap - EXIT

echo "== Homebrew that others can change is refused, and the rule goes away"
sudo mkdir -m 777 "$(brew --prefix)/furnish-open"
if out=$(as alice "$furnish" install brewdemo 2>&1); then fail "installed with Homebrew that others can change"; fi
echo "$out" | grep -q "only administrators can change" || fail "unexpected refusal: $out"
[ ! -e "$rule" ] || fail "$rule is still there"
if out=$(sudo "$furnish" service install --homebrew-sudo 2>&1); then fail "rule for Homebrew that others can change"; fi
echo "$out" | grep -q "everyone can write to\|others can write" || fail "unexpected refusal: $out"
sudo rmdir "$(brew --prefix)/furnish-open"
sudo "$furnish" service install --homebrew-sudo || fail "service install --homebrew-sudo failed"
[ -e "$rule" ] || fail "no rule"

sudo "$furnish" service uninstall || fail "service uninstall failed"
[ ! -e "$rule" ] || fail "$rule left behind"
[ ! -e "/Library/LaunchDaemons/$label.plist" ] || fail "launchd definition left behind"
if sudo launchctl print "system/$label" >/dev/null 2>&1; then fail "still loaded"; fi
echo "service OK"
