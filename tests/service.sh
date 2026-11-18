#!/bin/sh
# The furnish service in a container: a user without administrator rights installs an app the
# organization provides; other users can't; unsigned collection commits are refused.
#
#   tests/service.sh <path/to/furnish> [DSC resources folder]   (built for glibc ≤ the image's, e.g. in CI)
#
# With the folder of DSC's resources (scripts/fetch-dsc.mjs), the service also installs an app
# that uses them, from /usr/lib/furnish/dsc as in the Linux packages.
set -eu
if [ "${1:-}" != --inside ]; then
    binary=$(realpath "${1:?usage: $0 <furnish-binary> [dsc folder]}")
    repo=$(cd "$(dirname "$0")/.." && pwd)
    dsc=${2:+-v $(realpath "$2"):/dsc:ro}
    # shellcheck disable=SC2086 # $dsc is empty or two words
    exec docker run --rm -v "$repo:/src:ro" -v "$binary:/furnish-bin:ro" $dsc ubuntu:24.04 sh /src/tests/service.sh --inside
fi

fail() { echo "FAIL: $*" >&2; exit 1; }
as() { user=$1; shift; runuser -u "$user" -- env HOME="/home/$user" "$@"; }

apt-get update -qq && apt-get install -y -qq git openssh-client >/dev/null
# The mounted binary belongs to whoever built it; the service only runs one that root owns.
install -m 755 /furnish-bin /usr/bin/furnish
groupadd -f staff
useradd -m -G staff alice
useradd -m bob

# A fake pacman: only root can install, so a successful install proves the service did it.
mkdir -p /var/fake-pkgs
cat > /usr/local/bin/pacman <<'SH'
#!/bin/sh
for a do shift; [ "$a" = -- ] || set -- "$@" "$a"; done  # drop the end of options
case "$1" in
  -Q) [ -f "/var/fake-pkgs/$2" ] && echo "$2 1.0" ;;
  -Qu) exit 1 ;;
  -S) [ "$(id -u)" = 0 ] || { echo "pacman: you cannot perform this operation unless you are root" >&2; exit 1; }
      echo 1.0 > "/var/fake-pkgs/$4"
      if [ "$4" = brew ]; then cp /usr/local/libexec-brew /usr/local/bin/brew; fi ;;
  -R) [ "$(id -u)" = 0 ] || exit 1; rm "/var/fake-pkgs/$3"
      if [ "$3" = brew ]; then rm -f /usr/local/bin/brew; fi ;;
esac
SH
chmod +x /usr/local/bin/pacman
# A fake Homebrew, provided by installing the "brew" package; like the real one, it refuses root.
cat > /usr/local/libexec-brew <<'SH'
#!/bin/sh
[ "$(id -u)" != 0 ] || { echo "Error: Running Homebrew as root is extremely dangerous" >&2; exit 1; }
mkdir -p "$HOME/brew-pkgs"
case "$1" in
  list) [ -f "$HOME/brew-pkgs/$3" ] && echo "$3 1.0" ;;
  outdated) ;;
  install) touch "$HOME/brew-pkgs/$2" ;;
  uninstall) rm -f "$HOME/brew-pkgs/$2" ;;
esac
SH
chmod +x /usr/local/libexec-brew

# The organization's collection, signed with an SSH key the policy trusts.
ssh-keygen -q -t ed25519 -N '' -f /root/signing
git config --global user.name admin
git config --global user.email admin@example.com
git config --global gpg.format ssh
git config --global user.signingkey /root/signing
cp -r /src/tests/fixtures/collection /srv/org
mkdir -p /srv/org/managers
cat >> /srv/org/collection.yaml <<'YAML'
packageManagers:
  company: { script: managers/company.pl, interpreter: perl }
YAML
cat > /srv/org/managers/company.pl <<'PERL'
use strict;
use warnings;
use JSON::PP;
my $op = shift;
my $request = decode_json(<STDIN>);
my $marker = '/var/fake-pkgs/script-package';
my $want = $request->{_exist};
my $exists = -e $marker;
if ($op eq 'set' && $want != !!$exists) {
    if ($want) {
        open(my $file, '>', $marker) or die $!;
        print $file "$0\n";
        close($file) or die $!;
    } else { unlink($marker) or die $!; }
}
my $have = $op eq 'what-if' ? $want : -e $marker;
my %state = (name => $request->{name}, _exist => $have ? JSON::PP::true : JSON::PP::false);
$state{_inDesiredState} = $want == !!$have ? JSON::PP::true : JSON::PP::false if $op eq 'test';
print encode_json(\%state);
PERL
cat > /srv/org/apps/script-package.dsc.yaml <<'YAML'
resources:
  - name: Organization package
    type: Furnish/Package
    condition: "[contains(variables('furnish.host').packageManagers, 'company')]"
    properties: { name: script-package, packageManager: company }
YAML
cat > /srv/org/apps/mixed.dsc.yaml <<'YAML'
metadata:
  furnish:
    perUser: true
    uninstall:
      - name: remove-module
        type: Furnish/NixOS
        properties: { name: tool, _exist: false }
resources:
  - name: module
    type: Furnish/NixOS
    properties: { name: tool, packages: [hello] }
  - name: other-platform
    type: Furnish/NixOS
    condition: "[equals(context().os.family, 'Windows')]"
    properties: { name: tool }
  - name: my-file
    type: Furnish/File
    properties: { path: ~/.tool, content: tool }
YAML
cat > /srv/org/apps/guarded.dsc.yaml <<'YAML'
resources:
  - name: First step
    type: Furnish/File
    properties: { path: /var/furnish-preflight-first, content: ready }
  - name: Private settings
    type: Furnish/Value
    properties: { path: /root/preflight.json, location: '$.enabled', value: true, requiresClosed: [furnish-cache-test] }
YAML
if [ -d /dsc ]; then
    mkdir -p /usr/lib/furnish && cp -r /dsc /usr/lib/furnish/dsc
    printf 'metadata:\n  furnish:\n    categories: [Test]\nresources:\n  - name: file\n    type: Microsoft.Filesystem.File/Content\n    properties: {path: /etc/furnish-dsc-test, content: furnish}\n' > /srv/org/apps/dsc-file.dsc.yaml
fi
git -C /srv/org init -q -b main
git -C /srv/org add -A
git -C /srv/org commit -q -S -m apps

mkdir -p /etc/furnish
cat > /etc/furnish/policy.json <<JSON
{"collections": {"preset": [{"name": "org", "url": "/srv/org"}], "usersCanAdd": false,
                 "trustedKeys": ["$(cat /root/signing.pub)"]},
 "service": {"allowedGroup": "staff"},
 "parameters": {"org": {"role": {"value": "developer"}}}}
JSON

echo "== service install/uninstall register it with systemd and polkit"
printf '#!/bin/sh\necho "$*" >> /tmp/systemctl.log\n' > /usr/local/bin/systemctl && chmod +x /usr/local/bin/systemctl
install -o alice -m 755 /usr/bin/furnish /home/alice/furnish
if out=$(/home/alice/furnish service install 2>&1); then fail "installed a service binary that alice can change"; fi
echo "$out" | grep -q "not root" || fail "unexpected refusal: $out"
[ ! -e /etc/systemd/system/furnish.service ] || fail "unit written for alice's binary"
if as alice furnish service install 2>/dev/null; then fail "a user installed the service"; fi
furnish service install >/dev/null || fail "service install failed"
grep -q "^ExecStart=/usr/bin/furnish service$" /etc/systemd/system/furnish.service || fail "unit doesn't run this furnish"
grep -q 'io.github.furnish.install' /usr/share/polkit-1/actions/io.github.furnish.policy || fail "no polkit action"
grep -q "^enable furnish$" /tmp/systemctl.log && grep -q "^restart furnish$" /tmp/systemctl.log || fail "not enabled and started: $(cat /tmp/systemctl.log)"
furnish service uninstall >/dev/null || fail "service uninstall failed"
[ ! -e /etc/systemd/system/furnish.service ] && [ ! -e /usr/share/polkit-1/actions/io.github.furnish.policy ] || fail "files left behind"
grep -q "^disable --now furnish$" /tmp/systemctl.log || fail "not disabled"

furnish service 2>/tmp/service.log &
for _ in $(seq 50); do [ -S /run/furnish.sock ] && break; sleep 0.2; done
[ -S /run/furnish.sock ] || { cat /tmp/service.log; fail "service didn't start"; }

# A connection that never sends a request is closed after 30 s (checked at the end).
perl -MIO::Socket::UNIX -MTime::HiRes=time -e '
    my $s = IO::Socket::UNIX->new(Peer => "/run/furnish.sock") or die "connect: $!";
    my $t = time; 1 while <$s>; printf "%d\n", time - $t' > /tmp/idle.txt &
idle=$!

echo "== a member of the allowed group installs system-wide"
as alice furnish install hyperfine || { cat /tmp/service.log; fail "alice couldn't install"; }
[ -f /var/fake-pkgs/hyperfine ] || fail "not installed"
as alice furnish list | grep "✓  hyperfine" >/dev/null || fail "alice doesn't see it installed"
[ ! -e /home/alice/.local/share/furnish/collections/org ] || fail "alice got her own copy of the collection"

if [ -d /dsc ]; then
    echo "== an app with DSC's resources from /usr/lib/furnish/dsc"
    as alice furnish install dsc-file || { cat /tmp/service.log; fail "alice couldn't install dsc-file"; }
    [ "$(cat /etc/furnish-dsc-test)" = furnish ] && [ "$(stat -c %U /etc/furnish-dsc-test)" = root ] || fail "File/Content didn't run as root"
fi

echo "== collection manager runs from the service's signed checkout"
as alice furnish install org/script-package --dry-run >/dev/null || fail "could not preview the collection manager"
[ ! -e /var/fake-pkgs/script-package ] || fail "preview changed system state"
as alice furnish install org/script-package || { cat /tmp/service.log; fail "script package install failed"; }
[ "$(stat -c %U /var/fake-pkgs/script-package)" = root ] || fail "script did not install as the service"
[ "$(cat /var/fake-pkgs/script-package)" = /var/lib/furnish/collections/org/managers/company.pl ] || fail "wrong script checkout"
as alice furnish uninstall org/script-package || fail "script package uninstall failed"
[ ! -e /var/fake-pkgs/script-package ] || fail "script package is still installed"

echo "== others are refused"
if out=$(as bob furnish uninstall hyperfine 2>&1); then fail "bob uninstalled"; fi
echo "$out" | grep -q "aren't allowed" || fail "unexpected refusal: $out"
[ -f /var/fake-pkgs/hyperfine ] || fail "removed anyway"

echo "== requests can't smuggle in options or apps from elsewhere"
raw() {  # send one raw request as alice, print the last answer line
    as alice perl -MIO::Socket::UNIX -e '
        my $s = IO::Socket::UNIX->new(Peer => "/run/furnish.sock") or die "connect: $!";
        print $s "$ARGV[0]\n"; my $last; $last = $_ while <$s>; print $last' "$1"
}
raw '{"install":{"app":"org/hyperfine","values":{"prefix":"/tmp"}}}' | grep -q "unknown option 'prefix'" || fail "undeclared option accepted"
raw '{"install":{"app":"org/cli-tool","values":{"userInstall":"yes"}}}' | grep -q "needs a bool" || fail "wrong type accepted"
raw '{"install":{"app":"../../tmp/evil","values":{}}}' | grep -q '"error"' || fail "odd app name accepted"
raw '{"install":{"app":"org/hyperfine","values":{},"doc":"x"}}' | grep -q "invalid request" || fail "unknown field accepted"

echo "== the service preflights guarded settings using its own read permissions"
printf '{"enabled":false}' > /root/preflight.json
chmod 600 /root/preflight.json
cp /bin/sleep /tmp/furnish-cache-test
/tmp/furnish-cache-test 120 &
writer=$!
if out=$(as alice furnish install org/guarded 2>&1); then fail "changed settings while their writer was running"; fi
echo "$out" | grep -q "Close furnish-cache-test, then retry this change" || fail "no actionable service response: $out"
[ ! -e /var/furnish-preflight-first ] || fail "ran an earlier step while waiting"
raw '{"install":{"app":"org/guarded","values":{}}}' | grep -Fq '"waitingForClose":["furnish-cache-test"]' || fail "no process names in the service reply"
kill "$writer" && wait "$writer" || true
as alice furnish install org/guarded >/dev/null || fail "install after closing the writer failed"
[ "$(cat /var/furnish-preflight-first)" = ready ] && grep -q 'true' /root/preflight.json || fail "service did not apply the requested changes"

echo "== the service only applies the matching NixOS step of a preset per-user app"
scoped() {
    raw "{\"nixosStep\":{\"step\":{\"app\":\"org/mixed\",\"values\":{},\"step\":\"$1\",\"uninstall\":$2,\"fingerprint\":\"$3\"}}}"
}
scoped my-file false x | grep -q "no such Furnish/NixOS" || fail "accepted a user's file step"
scoped other-platform false x | grep -q "does not apply" || fail "accepted a step for another OS"
scoped module true x | grep -q "no such Furnish/NixOS" || fail "ignored the explicit removal steps"
scoped module false x | grep -q "differs from the current preset" || fail "accepted mismatched NixOS properties"
scoped remove-module true x | grep -q "differs from the current preset" || fail "didn't check the removal step"
# A matching, already-absent removal is accepted even on this machine without Nix.
properties='{"name":"tool","packages":[],"options":{},"homeOptions":{},"onlyIfSetUp":false,"_exist":false}'
fingerprint=$(printf %s "$properties" | openssl dgst -sha256 -binary | base64)
scoped remove-module true "$fingerprint" | grep -q '"done"' || fail "refused a matching removal"
raw '{"uninstall":{"app":"org/mixed"}}' | grep -q "without the service" || fail "accepted an entire per-user removal"

echo "== per-user installs run as the user; their dependencies go through the service"
as alice furnish option set org/cli-tool userInstall true >/dev/null
as alice furnish install cli-tool || { cat /tmp/service.log; fail "alice couldn't install just for herself"; }
[ -f /var/fake-pkgs/brew ] || fail "Homebrew (a system-wide dependency) wasn't installed"
[ -f /home/alice/brew-pkgs/cli-tool ] || fail "not installed into alice's account"
[ ! -f /var/fake-pkgs/cli-tool ] || fail "installed system-wide"
raw '{"install":{"app":"org/cli-tool","values":{"userInstall":true}}}' | grep -q "without the service" || fail "the service accepted a per-user install"
echo "== switching to a system-wide install moves it"
as alice furnish option set org/cli-tool userInstall false >/dev/null
as alice furnish reinstall cli-tool >/dev/null || fail "reinstall failed"
[ -f /var/fake-pkgs/cli-tool ] && [ ! -f /home/alice/brew-pkgs/cli-tool ] || fail "not moved to a system-wide install"
as alice furnish uninstall cli-tool >/dev/null || fail "couldn't uninstall"
[ ! -f /var/fake-pkgs/cli-tool ] || fail "still installed"

echo "== unsigned commits are refused"
old=$(git -C /var/lib/furnish/collections/org rev-parse HEAD)
echo "# unsigned" >> /srv/org/apps/hyperfine.dsc.yaml
git -C /srv/org commit -q -a --no-gpg-sign -m unsigned
if out=$(as alice furnish collection update org 2>&1); then fail "unsigned commit accepted"; fi
echo "$out" | grep -q "isn't signed" || fail "unexpected error: $out"
echo "== users who may not install don't see why an update failed; the service's log does"
if out=$(as bob furnish collection update org 2>&1); then fail "unsigned commit accepted for bob"; fi
echo "$out" | grep -q "details in its log" || fail "unexpected error for bob: $out"
if echo "$out" | grep -q "isn't signed"; then fail "bob sees the details: $out"; fi
grep -q "isn't signed" /tmp/service.log || fail "details not in the service's log"
[ "$(git -C /var/lib/furnish/collections/org rev-parse HEAD)" = "$old" ] || fail "moved to the unsigned commit"
git -C /srv/org commit -q --amend -S -m signed
as alice furnish collection update org >/dev/null || fail "signed commit refused"

echo "== choosing a version through the service; only signed ones; the policy can turn it off"
short() { echo "$1" | cut -c1-7; }
first=$(git -C /srv/org rev-list --max-parents=0 HEAD)
echo "# unsigned" >> /srv/org/apps/hyperfine.dsc.yaml
git -C /srv/org commit -q -a --no-gpg-sign -m "unsigned hyperfine" && unsigned=$(git -C /srv/org rev-parse HEAD)
echo "# signed" >> /srv/org/apps/hyperfine.dsc.yaml
git -C /srv/org commit -q -a -S -m "signed hyperfine"
as alice furnish collection update org >/dev/null || fail "a signed commit after an unsigned one was refused"
out=$(as alice furnish versions hyperfine 2>&1) || { cat /tmp/service.log; fail "alice couldn't list the versions: $out"; }
echo "$out" | grep -q "$(short "$first")" || fail "the first version isn't listed: $out"
echo "$out" | grep -q "$(short "$unsigned")" && fail "the unsigned version is listed: $out"
as alice furnish install hyperfine --version "$(short "$first")" >/dev/null || { cat /tmp/service.log; fail "alice couldn't install the first version"; }
as alice furnish list | grep hyperfine | grep -q "the version you chose" || fail "not held at the chosen version: $(as alice furnish list 2>&1)"
raw "{\"install\":{\"app\":\"org/hyperfine\",\"values\":{},\"which\":{\"version\":\"$unsigned\"}}}" | grep -q "isn't signed" || fail "the service installed an unsigned version"
if out=$(as bob furnish install hyperfine --version "$(short "$first")" 2>&1); then fail "bob chose a version"; fi
sed -i 's/"service":/"collectionSettings": {"org": {"chooseVersion": {"value": false, "locked": true}}}, "service":/' /etc/furnish/policy.json
raw "{\"install\":{\"app\":\"org/hyperfine\",\"values\":{},\"which\":{\"version\":\"$first\"}}}" | grep -q "turned off" || fail "the service installed a chosen version with choosing turned off"
raw '{"versions":{"app":"org/hyperfine"}}' | grep -q "turned off" || fail "the service listed versions with choosing turned off"
as alice furnish list | grep hyperfine | grep -q "held" && fail "still held with choosing turned off"
as alice furnish install hyperfine >/dev/null || fail "updating to the current version failed"
sed -i 's/"collectionSettings": {"org": {"chooseVersion": {"value": false, "locked": true}}}, //' /etc/furnish/policy.json

echo "== uninstall through the service"
as alice furnish uninstall hyperfine >/dev/null || fail "alice couldn't uninstall"
[ ! -f /var/fake-pkgs/hyperfine ] || fail "still installed"

echo "== idle connections are closed"
wait "$idle"
[ "$(cat /tmp/idle.txt)" -ge 25 ] && [ "$(cat /tmp/idle.txt)" -le 40 ] || fail "idle connection closed after $(cat /tmp/idle.txt) s"
echo "service OK"
