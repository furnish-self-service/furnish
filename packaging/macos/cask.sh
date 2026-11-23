#!/bin/sh
# Point the Homebrew cask (Casks/furnish.rb in furnish-self-service/homebrew-tap) at a .dmg the CI
# built: sets `version` and `sha256`, prints the version. The GitHub release for the version has
# the .dmg under the same name with "-" for "+" (0.1.0+alpha.42 is release v0.1.0-alpha.42).
#
#   packaging/macos/cask.sh <cask file> desktop/furnish_0.1.0+alpha.42_arm64.dmg
set -eu
cask=${1:?usage: $0 <cask file> <dmg>}; dmg=${2:?}
version=$(basename "$dmg" | sed -E 's/^furnish_(.+)_arm64\.dmg$/\1/; s/\+/-/g')
case "$version" in *.dmg) echo "unexpected file name: $dmg" >&2; exit 1 ;; esac
# Homebrew's default GitHub release strategy selects stable releases.
case "$version" in
    *-alpha.*) ;;
    *) sed '/^  # Alpha builds /,/^  end$/d' "$cask" | cat -s > "$cask.tmp"
       mv "$cask.tmp" "$cask"
       if grep -v '^  version ' "$cask" | grep -q -- '-alpha'; then echo 'Unexpected alpha-only cask configuration' >&2; exit 1; fi ;;
esac
sha=$(shasum -a 256 "$dmg" | cut -d' ' -f1)
sed -i.bak -E "s/^  version \".*\"$/  version \"$version\"/; s/^  sha256 \".*\"$/  sha256 \"$sha\"/" "$cask"
rm -f "$cask.bak"
grep -q "^  version \"$version\"$" "$cask" && grep -q "^  sha256 \"$sha\"$" "$cask" || { echo "$cask has no version or sha256 line to set" >&2; exit 1; }
echo "$version"
