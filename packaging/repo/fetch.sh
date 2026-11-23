#!/bin/sh
# Restore the previous Pages deployment, including the other channel's signed metadata.
set -eu
url=${1:?usage: $0 <base url> <site dir>}; site=${2:?}
mkdir -p "$site"
get() { curl --retry 4 --retry-all-errors -fsSL "$1" -o "$2"; }
# Only the original, alpha-only repository predates the full site inventory.
status=$(curl --retry 4 -sSL -w '%{http_code}' "$url/files.txt" -o "$site/files.txt")
case "$status" in
    200) inventory=files.txt ;;
    404) rm "$site/files.txt"; inventory=packages.txt; get "$url/packages.txt" "$site/$inventory"
         if grep -qv '^alpha/' "$site/$inventory"; then exit 1; fi ;;
    *) echo "Fetching the repository inventory returned HTTP $status" >&2; exit 1 ;;
esac
test -s "$site/$inventory"
while IFS= read -r file; do
    case "$file" in /*|*..*|'') exit 1 ;; esac
    mkdir -p "$site/$(dirname "$file")"
    get "$url/$file" "$site/$file"
done < "$site/$inventory"
