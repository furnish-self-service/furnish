#!/bin/sh
# Write <channel dir>/latest.json, the feed furnish's built-in updater reads (Tauri's format), for
# the newest version in the channel that has a signed download: windows/furnish_<v>_x64.msi and
# macos/furnish_<v>_arm64.app.tar.gz, each with its .sig (`tauri signer sign`).
#
#   packaging/repo/update-feed.sh <channel dir> <URL of the channel dir>
#
# The feed's version is SemVer (0.1.0+alpha.42 becomes 0.1.0-alpha.42), so versions compare
# correctly; it must match the version each signature names. Needs jq.
set -eu
dir=${1:?usage: $0 <channel dir> <its URL>}; url=${2:?usage: $0 <channel dir> <its URL>}
versions=$( { ls "$dir/windows" 2>/dev/null | sed -nE 's/^furnish_(.*)_x64\.msi\.sig$/\1/p'
              ls "$dir/macos" 2>/dev/null | sed -nE 's/^furnish_(.*)_arm64\.app\.tar\.gz\.sig$/\1/p'; } | sort -V)
version=$(echo "$versions" | tail -n 1)
if [ -z "$version" ]; then echo "no signed downloads in $dir"; exit 1; fi
semver=$(echo "$version" | sed 's/+alpha\./-alpha./')

platforms='{}'
add() { # <target> <file relative to the channel dir>
    if [ -f "$dir/$2" ] && [ -f "$dir/$2.sig" ]; then
        platforms=$(echo "$platforms" | jq --arg t "$1" --arg u "$url/$2" --arg s "$(cat "$dir/$2.sig")" '. + {($t): {url: $u, signature: $s}}')
    fi
}
add windows-x86_64 "windows/furnish_${version}_x64.msi"
add darwin-aarch64 "macos/furnish_${version}_arm64.app.tar.gz"
case "$semver" in
    *-alpha.*) notes="https://github.com/furnish-self-service/furnish/releases/tag/v$semver" ;;
    *) notes="https://github.com/furnish-self-service/furnish/releases" ;;
esac
jq -n --arg v "$semver" --arg n "$notes" --arg d "$(date -u +%Y-%m-%dT%H:%M:%SZ)" --argjson p "$platforms" \
    '{version: $v, notes: $n, pub_date: $d, platforms: $p}' > "$dir/latest.json"
echo "$dir/latest.json: $semver for $(echo "$platforms" | jq -r 'keys | join(", ")')"
