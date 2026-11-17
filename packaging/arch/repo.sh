#!/bin/sh
# Sign the packages in a pacman repository directory (a detached .sig each) and regenerate its
# signed database (furnish.db, furnish.db.sig), which lists the newest version (one per package).
# Needs repo-add (pacman) and gpg with the signing key.
#
#   packaging/arch/repo.sh <dir with .pkg.tar.zst>
set -eu
dir=${1:?usage: $0 <dir with .pkg.tar.zst>}
cd "$dir"
for pkg in *.pkg.tar.zst; do
    [ -f "$pkg.sig" ] || gpg --batch --yes --detach-sign -o "$pkg.sig" "$pkg"
done
rm -f furnish.db* furnish.files*
repo-add --quiet --sign furnish.db.tar.gz "$(ls *.pkg.tar.zst | sort -V | tail -n 1)"
# Plain files instead of repo-add's symlinks, for static hosting.
for f in furnish.db furnish.db.sig furnish.files furnish.files.sig; do
    cp --remove-destination "$(readlink -f "$f")" "$f"
done
