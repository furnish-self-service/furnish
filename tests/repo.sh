#!/bin/sh
# Add the furnish package repository the way the README says, install furnish and run it.
#
#   tests/repo.sh <image> <base url> <channel>    e.g. debian:12 https://furnish-self-service.github.io/furnish alpha
set -eu
image=${1:?usage: $0 <image> <base url> <channel>}; url=${2:?}; channel=${3:?}
case "$image" in
    debian*|ubuntu*) steps="apt-get update -qq && apt-get install -y -qq curl ca-certificates >/dev/null
        install -d /etc/apt/keyrings && curl -fsSL $url/gpg-key.asc -o /etc/apt/keyrings/furnish.asc
        curl -fsSL $url/$channel/furnish.list -o /etc/apt/sources.list.d/furnish.list
        apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq furnish >/dev/null" ;;
    fedora*) steps="curl -fsSL $url/$channel/furnish.repo -o /etc/yum.repos.d/furnish.repo
        dnf install -y -q furnish >/dev/null" ;;
    almalinux*|rockylinux*) steps="dnf install -y -q epel-release >/dev/null && dnf config-manager --set-enabled crb
        curl -fsSL $url/$channel/furnish.repo -o /etc/yum.repos.d/furnish.repo
        dnf install -y -q furnish >/dev/null" ;;
    opensuse*) steps="zypper -n -q --gpg-auto-import-keys addrepo $url/$channel/furnish.repo
        zypper -n -q --gpg-auto-import-keys install furnish >/dev/null" ;;
    *) echo "unknown image: $image" >&2; exit 1 ;;
esac
docker run --rm --network host "$image" sh -c "set -e
$steps
furnish --version && echo 'repository OK on $image'"
