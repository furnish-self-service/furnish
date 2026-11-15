#!/bin/sh
# Add packages to a channel of the furnish package repository and regenerate its signed metadata.
# The repository is a static site (GitHub Pages): fetch.sh first restores what it already serves.
#
#   BASE_URL=https://furnish-self-service.github.io/furnish packaging/repo/publish.sh alpha <dir with .deb/.rpm> <site dir>
#
# Needs apt-ftparchive (apt-utils), createrepo_c and gpg with the signing key; KEEP versions per
# channel (default 5) stay available.
set -eu
channel=${1:?usage: $0 <channel> <packages dir> <site dir>}
packages=${2:?}; site=${3:?}
: "${BASE_URL:?set BASE_URL to where the site is served}"
here=$(cd "$(dirname "$0")" && pwd)
apt="$site/$channel/apt"; rpm="$site/$channel/rpm"; mac="$site/$channel/macos"; win="$site/$channel/windows"
mkdir -p "$apt/pool/main" "$apt/dists/furnish/main/binary-amd64" "$rpm/x86_64" "$mac" "$win"
find "$packages" -name '*.deb' -exec cp {} "$apt/pool/main/" \;
find "$packages" -name '*.rpm' -exec cp {} "$rpm/x86_64/" \;
# The desktop app for macOS and Windows: downloads, no package manager (unsigned for now).
find "$packages" -name '*.dmg' -exec cp {} "$mac/" \;
find "$packages" -name '*.msi' -exec cp {} "$win/" \;

# Keep the newest versions only (GitHub Pages sites are limited to 1 GB).
prune() { ls "$1" | sort -V | head -n "-${KEEP:-5}" | while read -r f; do rm -f "$1/$f"; done; }
prune "$apt/pool/main"; prune "$rpm/x86_64"; prune "$mac"; prune "$win"
# Stable links to the newest download, for the website.
latest() { ls "$1" | grep -E "$2\$" | sort -V | tail -n 1; }
if [ -n "$(latest "$mac" .dmg)" ]; then cp "$mac/$(latest "$mac" .dmg)" "$site/$channel/furnish-macos.dmg"; fi
if [ -n "$(latest "$win" .msi)" ]; then cp "$win/$(latest "$win" .msi)" "$site/$channel/furnish-windows.msi"; fi
# The static Linux CLI on its own, e.g. for `furnish check` in a collection's CI (the packages contain it too).
if [ -f "$packages/cli/furnish" ]; then install -m 755 "$packages/cli/furnish" "$site/$channel/furnish-linux-x86_64"; fi

# APT: one suite for every Debian-based distro (the packages run on all of them).
(cd "$apt" && apt-ftparchive packages pool > dists/furnish/main/binary-amd64/Packages)
gzip -9kf "$apt/dists/furnish/main/binary-amd64/Packages"
apt-ftparchive -o APT::FTPArchive::Release::Origin=furnish -o APT::FTPArchive::Release::Label=furnish \
    -o APT::FTPArchive::Release::Suite=furnish -o APT::FTPArchive::Release::Codename=furnish \
    -o APT::FTPArchive::Release::Architectures=amd64 -o APT::FTPArchive::Release::Components=main \
    release "$apt/dists/furnish" > "$site/Release.tmp"
mv "$site/Release.tmp" "$apt/dists/furnish/Release"
gpg --batch --yes --clearsign -o "$apt/dists/furnish/InRelease" "$apt/dists/furnish/Release"
gpg --batch --yes --armor --detach-sign -o "$apt/dists/furnish/Release.gpg" "$apt/dists/furnish/Release"

# RPM: one repository for Fedora, openSUSE and Enterprise Linux; the metadata is signed.
rm -rf "$rpm/repodata"
createrepo_c --quiet "$rpm"
gpg --batch --yes --armor --detach-sign -o "$rpm/repodata/repomd.xml.asc" "$rpm/repodata/repomd.xml"

cp "$here/gpg-key.asc" "$site/gpg-key.asc"
cat > "$site/$channel/furnish.repo" <<REPO
[furnish-$channel]
name=furnish ($channel)
baseurl=$BASE_URL/$channel/rpm
enabled=1
gpgcheck=0
repo_gpgcheck=1
gpgkey=$BASE_URL/gpg-key.asc
REPO
echo "deb [signed-by=/etc/apt/keyrings/furnish.asc] $BASE_URL/$channel/apt furnish main" > "$site/$channel/furnish.list"

# What fetch.sh restores next time: the packages of every channel.
(cd "$site" && find . \( -name '*.deb' -o -name '*.rpm' -o -path '*/macos/*.dmg' -o -path '*/windows/*.msi' \) | sed 's|^\./||' | sort) > "$site/packages.txt"
echo "published $(ls "$apt/pool/main" | wc -l) .deb, $(ls "$rpm/x86_64" | wc -l) .rpm, $(ls "$mac" | wc -l) .dmg and $(ls "$win" | wc -l) .msi in $channel"
