#!/bin/sh
# Run a distro test's commands with its Docker options and mounts.
set -eu
image=$1
commands=$2
shift 2
case "$image" in
    opensuse/tumbleweed*)
        # Tumbleweed's redirector can return a mirror that lacks files named in the
        # signed repository index. Refresh the same OSS repo from openSUSE's CDN in that case.
        commands='
            if ! zypper -n -q refresh repo-oss; then
                echo "Retrying the Tumbleweed OSS repository from the openSUSE content CDN" >&2
                sed -i "s|^baseurl=.*|baseurl=https://downloadcontentcdn.opensuse.org/tumbleweed/repo/oss/|" /etc/zypp/repos.d/repo-oss.repo
                zypper -n refresh --force repo-oss
            fi
        '"$commands" ;;
esac
exec docker run --rm "$@" "$image" sh -ec "$commands"
