#!/bin/sh
# Restore the previous Pages deployment, including the other channel's signed metadata.
set -eu
url=${1:?usage: $0 <base url> <site dir>}; site=${2:?}
mkdir -p "$site"
restore() {
    case "/$1/" in *'/../'*|*'/./'*|*'//'*) echo "Invalid inventory path: $1" >&2; exit 1 ;; esac
    mkdir -p "$site/$(dirname "$1")"
    curl --retry 4 --retry-all-errors -fsSL "$url/$1" -o "$site/$1"
}
inventory() {
    code=$(curl --retry 4 -sSL -w '%{http_code}' "$url/$1" -o "$site/inventory.tmp") || exit 1
    case "$code" in
        200) test -s "$site/inventory.tmp" || exit 1; mv "$site/inventory.tmp" "$site/$1" ;;
        404) rm -f "$site/inventory.tmp"; return 1 ;;
        *) echo "Fetching $1 returned HTTP $code" >&2; exit 1 ;;
    esac
}
if inventory files.txt; then
    list=files.txt
elif inventory packages.txt; then
    list=packages.txt
    if grep -qv '^alpha/' "$site/$list"; then exit 1; fi
    # Legacy deployments inventoried only packages. Restore the rest of the alpha site too.
    for file in gpg-key.asc alpha/furnish-linux-x86_64 alpha/furnish-macos.dmg alpha/furnish-windows.msi \
        alpha/furnish.repo alpha/furnish.list alpha/latest.json alpha/rpm/repodata/repomd.xml \
        alpha/rpm/repodata/repomd.xml.asc; do restore "$file"; done
    for file in InRelease Release Release.gpg main/binary-amd64/Packages main/binary-amd64/Packages.gz; do
        restore "alpha/apt/dists/furnish/$file"
    done
    python3 -c 'import sys, xml.etree.ElementTree as ET
for loc in ET.parse(sys.argv[1]).iterfind(".//{*}location"):
    print("alpha/rpm/" + loc.attrib["href"])' "$site/alpha/rpm/repodata/repomd.xml" > "$site/rpm-files.tmp"
    while IFS= read -r file; do restore "$file"; done < "$site/rpm-files.tmp"
    rm "$site/rpm-files.tmp"
else
    echo "No existing Pages repository"; exit 0
fi
while IFS= read -r file; do restore "$file"; done < "$site/$list"
echo "Restored $(wc -l < "$site/$list") inventory entries"
