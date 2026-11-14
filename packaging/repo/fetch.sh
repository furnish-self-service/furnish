#!/bin/sh
# Restore the packages the published repository serves (listed in packages.txt), so a new
# publish keeps older versions and other channels. A site that doesn't exist yet is empty.
#
#   packaging/repo/fetch.sh https://furnish-self-service.github.io/furnish <site dir>
set -eu
url=${1:?usage: $0 <base url> <site dir>}; site=${2:?}
mkdir -p "$site"
curl -fsS "$url/packages.txt" -o "$site/packages.txt" || { echo "no published repository yet"; exit 0; }
while read -r file; do
    mkdir -p "$site/$(dirname "$file")"
    curl -fsS "$url/$file" -o "$site/$file"
done < "$site/packages.txt"
echo "restored $(wc -l < "$site/packages.txt") packages"
