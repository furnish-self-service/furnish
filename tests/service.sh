#!/bin/sh
# The furnish service in a container: a user without administrator rights installs an app the
# organization provides; other users can't; unsigned collection commits are refused.
#
#   tests/service.sh <path/to/furnish>      (built for glibc ≤ the image's, e.g. in CI)
set -eu
if [ "${1:-}" != --inside ]; then
    binary=$(realpath "${1:?usage: $0 <furnish-binary>}")
    repo=$(cd "$(dirname "$0")/.." && pwd)
    exec docker run --rm -v "$repo:/src:ro" -v "$binary:/usr/bin/furnish:ro" ubuntu:24.04 sh /src/tests/service.sh --inside
fi

fail() { echo "FAIL: $*" >&2; exit 1; }
as() { user=$1; shift; runuser -u "$user" -- env HOME="/home/$user" "$@"; }

apt-get update -qq && apt-get install -y -qq git openssh-client >/dev/null
groupadd -f staff
useradd -m -G staff alice
useradd -m bob

# A fake pacman: only root can install, so a successful install proves the service did it.
mkdir -p /var/fake-pkgs
cat > /usr/local/bin/pacman <<'SH'
#!/bin/sh
case "$1" in
  -Q) [ -f "/var/fake-pkgs/$2" ] && echo "$2 1.0" ;;
  -Qu) exit 1 ;;
  -S) [ "$(id -u)" = 0 ] || { echo "pacman: you cannot perform this operation unless you are root" >&2; exit 1; }
      echo 1.0 > "/var/fake-pkgs/$4" ;;
  -R) [ "$(id -u)" = 0 ] || exit 1; rm "/var/fake-pkgs/$3" ;;
esac
SH
chmod +x /usr/local/bin/pacman

# The organization's collection, signed with an SSH key the policy trusts.
ssh-keygen -q -t ed25519 -N '' -f /root/signing
git config --global user.name admin
git config --global user.email admin@example.com
git config --global gpg.format ssh
git config --global user.signingkey /root/signing
cp -r /src/tests/fixtures/collection /srv/org
git -C /srv/org init -q -b main
git -C /srv/org add -A
git -C /srv/org commit -q -S -m apps

mkdir -p /etc/furnish
cat > /etc/furnish/policy.json <<JSON
{"collections": {"preset": [{"name": "org", "url": "/srv/org"}], "usersCanAdd": false,
                 "trustedKeys": ["$(cat /root/signing.pub)"]},
 "service": {"allowedGroup": "staff"}}
JSON

furnish service 2>/tmp/service.log &
for _ in $(seq 50); do [ -S /run/furnish.sock ] && break; sleep 0.2; done
[ -S /run/furnish.sock ] || { cat /tmp/service.log; fail "service didn't start"; }

echo "== a member of the allowed group installs system-wide"
as alice furnish install hyperfine || { cat /tmp/service.log; fail "alice couldn't install"; }
[ -f /var/fake-pkgs/hyperfine ] || fail "not installed"
as alice furnish list | grep "✓  hyperfine" >/dev/null || fail "alice doesn't see it installed"
[ ! -e /home/alice/.local/share/furnish/collections/org ] || fail "alice got her own copy of the collection"

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

echo "== unsigned commits are refused"
old=$(git -C /var/lib/furnish/collections/org rev-parse HEAD)
echo "# unsigned" >> /srv/org/apps/hyperfine.dsc.yaml
git -C /srv/org commit -q -a --no-gpg-sign -m unsigned
if out=$(as alice furnish collection update org 2>&1); then fail "unsigned commit accepted"; fi
echo "$out" | grep -q "isn't signed" || fail "unexpected error: $out"
[ "$(git -C /var/lib/furnish/collections/org rev-parse HEAD)" = "$old" ] || fail "moved to the unsigned commit"
git -C /srv/org commit -q --amend -S -m signed
as alice furnish collection update org >/dev/null || fail "signed commit refused"

echo "== uninstall through the service"
as alice furnish uninstall hyperfine >/dev/null || fail "alice couldn't uninstall"
[ ! -f /var/fake-pkgs/hyperfine ] || fail "still installed"
echo "service OK"
