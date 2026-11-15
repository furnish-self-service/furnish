#!/bin/sh
# Disk use and fallback: blobless and shallow checkouts, installer files downloaded when their app
# is installed, and installing the previous version of an app after a failure. Runs in Docker as
# root, with a small .deb as the installer.
#
#   tests/checkout.sh <path/to/furnish>   (a binary that runs on Debian trixie)
set -eu
if [ "${1:-}" != --inside ]; then
    binary=$(realpath "${1:?usage: $0 <furnish-binary>}")
    here=$(cd "$(dirname "$0")" && pwd)
    docker build -q -t furnish-git-auth "$here/git-auth" >/dev/null
    exec docker run --rm -v "$binary:/usr/local/bin/furnish:ro" -v "$here/checkout.sh:/t.sh:ro" furnish-git-auth sh /t.sh --inside
fi

fail() { echo "FAIL: $*" >&2; exit 1; }
export FURNISH_DATA_DIR=/tmp/data
git config --global user.name t; git config --global user.email t@t; git config --global init.defaultBranch main
# A file:// server that allows partial clones, like GitHub and GitLab do.
git config --global uploadpack.allowFilter true; git config --global uploadpack.allowAnySHA1InWant true

deb() { # deb <version> [package] [file] [broken]: a .deb in installers/; broken ones fail to install
    rm -rf /tmp/pkg && mkdir -p /tmp/pkg/DEBIAN /tmp/org/installers
    printf 'Package: furnish-test-%s\nVersion: %s\nArchitecture: all\nMaintainer: t <t@t>\nDescription: test\n' "${2:-tool}" "$1" > /tmp/pkg/DEBIAN/control
    if [ -n "${4:-}" ]; then printf '#!/bin/sh\nexit 1\n' > /tmp/pkg/DEBIAN/preinst && chmod 755 /tmp/pkg/DEBIAN/preinst; fi
    dpkg-deb -b /tmp/pkg "/tmp/org/installers/${3:-tool.deb}" >/dev/null
}
app() { # app <installer file> [app]
    printf 'metadata: { furnish: { description: Tool, categories: [Test] } }\nresources:\n  - name: tool\n    type: Furnish/Installer\n    properties: { file: %s }\n' "$1" > "/tmp/org/apps/${2:-tool}.dsc.yaml"
}
commit() { cd /tmp/org && git add -A && git commit -qm "$1" && cd /; }
mkdir -p /tmp/org/apps && cd /tmp/org && git init -q
deb 1.0; app installers/tool.deb
head -c 2000000 /dev/urandom > installers/other.msi
git add -A && git commit -qm v1 && v1=$(git rev-parse HEAD)
for i in 1 2 3; do echo "$i" > notes.txt; git add -A; git commit -qm "notes $i"; done

echo "== blobless, installers left out"
furnish collection add file:///tmp/org --name org >/dev/null
c=$FURNISH_DATA_DIR/collections/org
[ "$(git -C "$c" config remote.origin.promisor)" = true ] || fail "not a partial clone"
[ -f "$c/apps/tool.dsc.yaml" ] || fail "app file not checked out"
[ ! -e "$c/installers/tool.deb" ] && [ ! -e "$c/installers/other.msi" ] || fail "installers checked out"
furnish list | grep -q "·  tool" || fail "tool not listed as not installed: $(furnish list 2>&1)"
furnish install tool --dry-run | grep -q "download tool.deb" || fail "dry run doesn't mention the download: $(furnish install tool --dry-run 2>&1)"

echo "== installing checks out its installer"
furnish install tool >/dev/null || fail "install failed"
dpkg -s furnish-test-tool >/dev/null 2>&1 || fail "package not installed"
[ -f "$c/installers/tool.deb" ] || fail "installer not checked out"
[ ! -e "$c/installers/other.msi" ] || fail "an unrelated installer was checked out"
furnish list | grep -q "✓  tool" || fail "not shown as installed"

echo "== updates of a checked-out installer arrive"
cd /tmp/org && deb 2.0 && git add -A && git commit -qm v2 && cd /
furnish collection update org >/dev/null
furnish list | grep -q "↑  tool" || fail "no update shown: $(furnish list 2>&1)"
furnish install tool >/dev/null && [ "$(dpkg-query -W -f '${Version}' furnish-test-tool)" = 2.0 ] || fail "update not installed"

echo "== shallow: only the newest commit"
mkdir -p "$FURNISH_DATA_DIR" && printf '{"collections": {"shallow": {"checkoutDepth": 1}}}' > "$FURNISH_DATA_DIR/settings.json"
furnish collection add file:///tmp/org --name shallow >/dev/null
s=$FURNISH_DATA_DIR/collections/shallow
[ "$(git -C "$s" rev-list --count HEAD)" = 1 ] || fail "more than one commit: $(git -C "$s" rev-list --count HEAD)"
cd /tmp/org && echo 4 > notes.txt && git commit -qam "notes 4" && cd /
furnish collection update shallow >/dev/null
[ "$(git -C "$s" rev-list --count HEAD)" = 1 ] || fail "update deepened the history"
[ "$(git -C "$s" rev-parse HEAD)" = "$(git -C /tmp/org rev-parse HEAD)" ] || fail "update didn't arrive"

echo "== full copy: everything checked out"
printf '{"blobsOnDemand": false}' > "$FURNISH_DATA_DIR/settings.json"
furnish collection add file:///tmp/org --name full >/dev/null
[ -f "$FURNISH_DATA_DIR/collections/full/installers/other.msi" ] || fail "full copy lacks the installers"
[ -z "$(git -C "$FURNISH_DATA_DIR/collections/full" config remote.origin.promisor)" ] || fail "full copy is a partial clone"
rm "$FURNISH_DATA_DIR/settings.json"; furnish collection remove shallow >/dev/null; furnish collection remove full >/dev/null

echo "== a failed update offers the version that last installed here"
deb 3.0 tool tool3.deb broken; app installers/tool3.deb; commit "v3, broken"
furnish collection update org >/dev/null
out=$(furnish install tool 2>&1) && fail "the broken version installed"
echo "$out" | grep -q "furnish install tool --previous" || fail "no previous version offered: $out"
furnish install tool --previous >/dev/null || fail "installing the previous version failed"
[ "$(dpkg-query -W -f '${Version}' furnish-test-tool)" = 2.0 ] || fail "not at 2.0"
furnish list | grep "tool" | grep -q "held at an earlier version" || fail "not shown as held: $(furnish list 2>&1)"
furnish list | grep -q "✓  tool" || fail "held app not shown as installed"
furnish upgrade | grep -q "Nothing to update" || fail "upgrade retries the version that failed"

echo "== the hold ends when the collection changes the app"
deb 4.0; app installers/tool.deb; commit "v4, fixed"
furnish collection update org >/dev/null
furnish list | grep "tool" | grep -q "held" && fail "still held"
furnish install tool >/dev/null && [ "$(dpkg-query -W -f '${Version}' furnish-test-tool)" = 4.0 ] || fail "4.0 not installed"

echo "== first install, automatic: the app's version before its latest change"
deb 1.0 fresh fresh1.deb; app installers/fresh1.deb fresh; commit "fresh 1"
deb 2.0 fresh fresh2.deb broken; app installers/fresh2.deb fresh; commit "fresh 2, broken"
furnish collection update org >/dev/null
furnish collection settings org --fallback automatic | grep -q "install the previous version" || fail "setting not shown"
furnish install fresh >/dev/null 2>&1 || fail "automatic fallback failed"
[ "$(dpkg-query -W -f '${Version}' furnish-test-fresh)" = 1.0 ] || fail "fresh 1.0 not installed"
[ -f "$FURNISH_DATA_DIR/versions/org/$(git -C /tmp/org rev-parse HEAD~1)/installers/fresh1.deb" ] || fail "earlier installer not kept"
[ ! -e "$c/installers/fresh1.deb" ] || fail "the earlier installer was checked out into the collection"

echo "== maximum age"
deb 1.0 aged aged1.deb; app installers/aged1.deb aged
cd /tmp/org && git add -A && GIT_COMMITTER_DATE="$(date -d '10 days ago' -R)" git commit -qm "aged 1" && cd /
deb 2.0 aged aged2.deb broken; app installers/aged2.deb aged; commit "aged 2, broken"
furnish collection update org >/dev/null
furnish collection settings org --fallback ask --fallback-max-age 7 >/dev/null
out=$(furnish install aged --previous 2>&1) && fail "a version older than 7 days was installed"
echo "$out" | grep -q "more than 7 days older" || fail "unexpected message: $out"

echo "== admins can lock it off"
mkdir -p /etc/furnish && printf '{"collectionSettings": {"org": {"fallback": {"value": "off", "locked": true}}}}' > /etc/furnish/policy.json
out=$(furnish install aged --previous 2>&1) && fail "fallback ran while locked off"
echo "$out" | grep -q "turned off" || fail "unexpected message: $out"
out=$(furnish collection settings org --fallback ask 2>&1) && fail "a locked setting changed"
echo "$out" | grep -q "set by your organization" || fail "unexpected message: $out"
rm /etc/furnish/policy.json

echo "== Git LFS: installers downloaded when their app is installed"
mkdir -p /tmp/lfs/apps /tmp/lfs/installers && cd /tmp/lfs && git init -q && git lfs install --local >/dev/null && git lfs track '*.deb' >/dev/null
cp /tmp/org/installers/fresh1.deb installers/ && head -c 1000000 /dev/urandom > installers/big.deb
printf 'metadata: { furnish: { description: Fresh } }\nresources:\n  - name: fresh\n    type: Furnish/Installer\n    properties: { file: installers/fresh1.deb }\n' > apps/lfsapp.dsc.yaml
git add -A && git commit -qm lfs && cd /
dpkg -r furnish-test-fresh >/dev/null
furnish collection add file:///tmp/lfs --name lfs >/dev/null || fail "adding the LFS collection failed"
l=$FURNISH_DATA_DIR/collections/lfs
[ ! -e "$l/installers/fresh1.deb" ] && [ ! -e "$l/installers/big.deb" ] || fail "LFS installers checked out"
[ -z "$(ls "$l/.git/lfs/objects" 2>/dev/null)" ] || fail "LFS objects downloaded: $(du -sh "$l/.git/lfs/objects")"
furnish install lfsapp >/dev/null || fail "installing from LFS failed"
dpkg-deb -f "$l/installers/fresh1.deb" Package >/dev/null 2>&1 || fail "the LFS installer is a pointer, not the file"
[ ! -e "$l/installers/big.deb" ] || fail "an unrelated LFS file was checked out"

echo "checkout OK"
