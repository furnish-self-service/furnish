#!/bin/sh
# Print the version shared by the release binaries and package filenames.
set -eu
version=$(jq -er .version gui/src-tauri/tauri.conf.json)
if [ "${GITHUB_REF:-}" = refs/heads/main ] && [ "${RELEASE_CHANNEL:-}" = alpha ]; then
    version="$version+alpha.${GITHUB_RUN_NUMBER:?}"
fi
printf '%s\n' "$version"
