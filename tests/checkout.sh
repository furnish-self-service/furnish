#!/bin/sh
# Disk use and fallback: blobless and shallow checkouts, installer files downloaded when their app
# is installed, installing the previous version of an app after a failure, and choosing a version. Runs in Docker as
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
# Behind a symlink, like /tmp on macOS: paths furnish resolves must still match.
mkdir -p /tmp/real-data && ln -s /tmp/real-data /tmp/data
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
[ -z "$(ls "$FURNISH_DATA_DIR/versions/org" 2>/dev/null)" ] || fail "files of the earlier version left behind: $(ls "$FURNISH_DATA_DIR/versions/org")"

echo "== no earlier version while the current one installs"
out=$(furnish install tool --previous 2>&1) && fail "went back to an earlier version without a failure"
echo "$out" | grep -q "only after installing its current version failed" || fail "unexpected message: $out"
[ "$(dpkg-query -W -f '${Version}' furnish-test-tool)" = 4.0 ] || fail "not at 4.0 anymore"

echo "== choose an earlier version: kept when the collection changes the app"
short() { echo "$1" | cut -c1-7; }
v2=$(git -C /tmp/org log --format=%H --grep='^v2$')
v3=$(git -C /tmp/org log --format=%H --grep='^v3, broken$')
out=$(furnish versions tool 2>&1) || fail "listing the versions failed: $out"
[ "$(echo "$out" | grep -c "$(date +%F)")" = 4 ] || fail "not the four versions of tool: $out"
echo "$out" | grep "$(short "$(git -C /tmp/org rev-parse HEAD)")" | grep "✓" | grep -q current || fail "the current version isn't marked installed: $out"
echo "$out" | grep -q "$(short "$(git -C /tmp/org rev-parse HEAD~1)")" && fail "a commit that didn't change tool is listed: $out"
out=$(furnish install tool --version "$(date +%F)" 2>&1) && fail "a day with several versions installed one: $out"
echo "$out" | grep -q "give the commit instead" || fail "unexpected message: $out"
furnish install tool --version "$(short "$v2")" >/dev/null || fail "installing the version from v2 failed"
[ "$(dpkg-query -W -f '${Version}' furnish-test-tool)" = 2.0 ] || fail "not at 2.0"
furnish list | grep "tool" | grep -q "the version you chose" || fail "not shown as chosen: $(furnish list 2>&1)"
furnish versions tool | grep "$(short "$v2")" | grep -q "✓" || fail "the chosen version isn't marked installed: $(furnish versions tool)"
furnish upgrade | grep -q "Nothing to update" || fail "upgrade moves a chosen version"
deb 5.0; app installers/tool.deb; commit "v5"
furnish collection update org >/dev/null
furnish list | grep "tool" | grep -q "the version you chose" || fail "a change to the app ended the hold: $(furnish list 2>&1)"
furnish upgrade | grep -q "Nothing to update" || fail "upgrade moved a chosen version after a change"
[ "$(dpkg-query -W -f '${Version}' furnish-test-tool)" = 2.0 ] || fail "not at 2.0 anymore"
[ -n "$(ls "$FURNISH_DATA_DIR/versions/org/$v2" 2>/dev/null)" ] || fail "files of the chosen version not kept"
out=$(furnish install tool --version "$(short "$v3")" 2>&1) && fail "the broken version installed"
furnish list | grep "tool" | grep -q "the version you chose" || fail "a failed choice ended the hold"
furnish install tool --current >/dev/null || fail "going back to the current version failed"
[ "$(dpkg-query -W -f '${Version}' furnish-test-tool)" = 5.0 ] || fail "not at 5.0"
furnish list | grep "tool" | grep -q "held" && fail "still held after --current"
[ -z "$(ls "$FURNISH_DATA_DIR/versions/org" 2>/dev/null)" ] || fail "files of the chosen version left behind: $(ls "$FURNISH_DATA_DIR/versions/org")"
furnish collection settings org --choose-version false | grep -q "choose a version:  no" || fail "setting not shown"
out=$(furnish install tool --version "$(short "$v2")" 2>&1) && fail "a version was chosen while choosing is off"
echo "$out" | grep -q "turned off" || fail "unexpected message: $out"
furnish install tool --version "$(short "$(git -C /tmp/org rev-parse HEAD)")" >/dev/null || fail "the current version is refused while choosing is off"
furnish collection settings org --choose-version true >/dev/null

echo "== first install, automatic: the app's version before its latest change"
deb 1.0 fresh fresh1.deb; app installers/fresh1.deb fresh; commit "fresh 1"
deb 2.0 fresh fresh2.deb broken; app installers/fresh2.deb fresh; commit "fresh 2, broken"
furnish collection update org >/dev/null
furnish collection settings org --fallback automatic | grep -q "install the previous version" || fail "setting not shown"
furnish install fresh >/dev/null 2>&1 || fail "automatic fallback failed"
[ "$(dpkg-query -W -f '${Version}' furnish-test-fresh)" = 1.0 ] || fail "fresh 1.0 not installed"
[ -f "$FURNISH_DATA_DIR/versions/org/$(git -C /tmp/org rev-parse HEAD~1)/installers/fresh1.deb" ] || fail "earlier installer not kept"
[ ! -e "$c/installers/fresh1.deb" ] || fail "the earlier installer was checked out into the collection"

echo "== no fallback to a version the required versions don't accept"
deb 1.0 secure secure1.deb; app installers/secure1.deb secure; commit "secure 1"
deb 2.0 secure secure2.deb broken; app installers/secure2.deb secure
sed -i 's/{ file: \(.*\) }/{ file: \1, versions: [">=2"] }/' /tmp/org/apps/secure.dsc.yaml; commit "secure 2 requires 2.0, broken"
furnish collection update org >/dev/null
out=$(furnish install secure 2>&1) && fail "a broken or unacceptable version installed: $out"
[ "$(dpkg-query -W -f '${Version}' furnish-test-secure 2>/dev/null)" = 1.0 ] && fail "fell back to 1.0, which versions don't accept: $out"
out=$(furnish install secure --previous 2>&1) && fail "installed the previous version: $out"
echo "$out" | grep -q "has no acceptable version: needs 2 or later; the collection's file has 1.0" || fail "unexpected message: $out"
secure1=$(git -C /tmp/org log --format=%H --grep='^secure 1$')
out=$(furnish versions secure 2>&1) || fail "listing failed: $out"
echo "$out" | grep -q "$(short "$secure1")" && fail "a version without an acceptable version is listed: $out"
echo "$out" | grep -q "2 or later" || fail "the stated versions aren't shown: $out"
out=$(furnish install secure --version "$(short "$secure1")" 2>&1) && fail "chose a version without an acceptable version: $out"
echo "$out" | grep -q "has no acceptable version" || fail "unexpected message: $out"

echo "== maximum age"
deb 1.0 aged aged1.deb; app installers/aged1.deb aged
cd /tmp/org && git add -A && GIT_COMMITTER_DATE="$(date -d '10 days ago' -R)" git commit -qm "aged 1" && cd /
deb 2.0 aged aged2.deb broken; app installers/aged2.deb aged; commit "aged 2, broken"
furnish collection update org >/dev/null
furnish collection settings org --fallback ask --fallback-max-age 7 >/dev/null
out=$(furnish install aged --previous 2>&1) && fail "a version older than 7 days was installed"
echo "$out" | grep -q "more than 7 days older" || fail "unexpected message: $out"
aged1=$(git -C /tmp/org log --format=%H --grep='^aged 1$')
furnish versions aged | grep -q "$(short "$aged1")" && fail "a version older than 7 days is listed"
out=$(furnish install aged --version "$(short "$aged1")" 2>&1) && fail "chose a version older than 7 days"
echo "$out" | grep -q "more than 7 days older" || fail "unexpected message: $out"

echo "== admins can lock it off"
furnish install tool --version "$(short "$v2")" >/dev/null || fail "choosing the version from v2 again failed"
mkdir -p /etc/furnish && printf '{"collectionSettings": {"org": {"fallback": {"value": "off", "locked": true}, "chooseVersion": {"value": false, "locked": true}}}}' > /etc/furnish/policy.json
out=$(furnish versions tool 2>&1) && fail "versions listed while choosing is locked off"
echo "$out" | grep -q "turned off" || fail "unexpected message: $out"
furnish list | grep "tool" | grep -q "held" && fail "still held at the chosen version while choosing is off"
furnish list | grep -q "↑  tool" || fail "the current version isn't offered as an update: $(furnish list 2>&1)"
out=$(furnish collection settings org --choose-version true 2>&1) && fail "a locked setting changed"
out=$(furnish install aged --previous 2>&1) && fail "fallback ran while locked off"
echo "$out" | grep -q "turned off" || fail "unexpected message: $out"
out=$(furnish collection settings org --fallback ask 2>&1) && fail "a locked setting changed"
echo "$out" | grep -q "set by your organization" || fail "unexpected message: $out"
rm /etc/furnish/policy.json
furnish list | grep "tool" | grep -q "the version you chose" || fail "the hold didn't come back with choosing on"
furnish install tool --current >/dev/null

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

echo "== concurrent updates and clones of the same collection"
cd /tmp/org && echo 5 > notes.txt && git commit -qam "notes 5" && cd /
for i in 1 2 3 4; do furnish collection update org >/dev/null 2>/tmp/update$i.log & done; wait
for i in 1 2 3 4; do [ ! -s /tmp/update$i.log ] || fail "concurrent update $i: $(cat /tmp/update$i.log)"; done
[ "$(git -C "$c" rev-parse HEAD)" = "$(git -C /tmp/org rev-parse HEAD)" ] || fail "update didn't arrive"
git -C "$c" diff --quiet HEAD || fail "the checkout differs from its commit"
mkdir -p /etc/furnish && printf '{"collections": {"preset": [{"name": "twice", "url": "file:///tmp/org"}]}}' > /etc/furnish/policy.json
for i in 1 2 3 4; do furnish list >/dev/null 2>/tmp/list$i.log & done; wait
for i in 1 2 3 4; do grep -v "^note:" /tmp/list$i.log | grep -q . && fail "concurrent clone $i: $(cat /tmp/list$i.log)"; done
[ "$(git -C "$FURNISH_DATA_DIR/collections/twice" rev-parse HEAD)" = "$(git -C /tmp/org rev-parse HEAD)" ] || fail "preset not cloned"
[ -z "$(ls "$FURNISH_DATA_DIR/collections/.partial")" ] || fail "left in .partial: $(ls "$FURNISH_DATA_DIR/collections/.partial")"

echo "== a preset's name with another URL: replaced, not used as the preset"
furnish collection remove lfs >/dev/null
furnish collection add file:///tmp/lfs --name corp >/dev/null
printf '{"collections": {"preset": [{"name": "corp", "url": "file:///tmp/org"}]}}' > /etc/furnish/policy.json
furnish list >/dev/null 2>&1
[ "$(git -C "$FURNISH_DATA_DIR/collections/corp" config remote.origin.url)" = file:///tmp/org ] || fail "corp still comes from elsewhere"
furnish collection list | grep "^corp" | grep -q "managed by your organization" || fail "corp not managed: $(furnish collection list)"
out=$(furnish collection add file:///tmp/lfs --name corp 2>&1) && fail "added a collection with a preset's name"
echo "$out" | grep -q "your organization provides" || fail "unexpected message: $out"

echo "== trusted keys: SSH signatures only, no going back"
ssh-keygen -q -t ed25519 -N '' -f /root/signing
mkdir -p /tmp/signed/apps && cd /tmp/signed && git init -q
git config gpg.format ssh && git config user.signingkey /root/signing
cp /tmp/org/apps/tool.dsc.yaml apps/ && git add -A && git commit -qS -m "signed 1" && signed1=$(git rev-parse HEAD)
echo 2 > notes.txt && git add -A && git commit -qS -m "signed 2" && signed2=$(git rev-parse HEAD)
cd /
printf '{"collections": {"preset": [{"name": "signed", "url": "file:///tmp/signed"}], "trustedKeys": ["%s"]}}' "$(cat /root/signing.pub)" > /etc/furnish/policy.json
furnish collection list | grep "^signed" | grep -q managed || fail "the signed preset wasn't set up: $(furnish collection list 2>&1)"
sc=$FURNISH_DATA_DIR/collections/signed
# A key in root's GnuPG keyring, which git would use for an OpenPGP signature.
gpg -q --batch --passphrase '' --quick-gen-key 'Someone <someone@example.com>' ed25519 sign never 2>/dev/null
cd /tmp/signed && echo 3 > notes.txt && git add -A && git -c gpg.format=openpgp -c user.signingkey=someone@example.com commit -qS -m "gpg signed" && cd /
git -C /tmp/signed verify-commit HEAD 2>/dev/null || fail "test setup: git doesn't accept the GnuPG signature itself"
out=$(furnish collection update signed 2>&1) && fail "an OpenPGP-signed commit was accepted"
echo "$out" | grep -q "SSH signatures only" || fail "unexpected message: $out"
[ "$(git -C "$sc" rev-parse HEAD)" = "$signed2" ] || fail "moved to the OpenPGP-signed commit"
cd /tmp/signed && git reset -q --hard "$signed1" && cd /
out=$(furnish collection update signed 2>&1) && fail "went back to an earlier signed commit"
echo "$out" | grep -q "doesn't follow the current one" || fail "unexpected message: $out"
[ "$(git -C "$sc" rev-parse HEAD)" = "$signed2" ] || fail "moved back"
cd /tmp/signed && git reset -q --hard "$signed2" && echo 4 > notes.txt && git add -A && git commit -qS -m "signed 3" && cd /
furnish collection update signed >/dev/null || fail "a signed commit that follows was refused"
[ "$(git -C "$sc" rev-parse HEAD)" = "$(git -C /tmp/signed rev-parse HEAD)" ] || fail "signed commit didn't arrive"
echo "== trusted keys: only signed versions can be chosen"
cd /tmp/signed && sed -i 's/description: Tool/description: Tool 2/' apps/tool.dsc.yaml && git commit -qam "unsigned tool" && unsigned=$(git rev-parse HEAD)
sed -i 's/description: Tool 2/description: Tool 3/' apps/tool.dsc.yaml && git commit -qS -am "signed tool" && cd /
furnish collection update signed >/dev/null || fail "a signed commit after an unsigned one was refused"
out=$(furnish versions signed/tool 2>&1) || fail "listing failed: $out"
echo "$out" | grep -q "$(short "$signed1")" || fail "the signed version isn't listed: $out"
echo "$out" | grep -q "$(short "$unsigned")" && fail "the unsigned version is listed: $out"
out=$(furnish install signed/tool --version "$(short "$unsigned")" 2>&1) && fail "chose an unsigned version"
echo "$out" | grep -q "isn't signed" || fail "unexpected message: $out"
rm /etc/furnish/policy.json

echo "checkout OK"
