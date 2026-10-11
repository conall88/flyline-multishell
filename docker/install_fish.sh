#!/bin/sh
# Install the fish shell a test image runs against.
#
#   install_fish.sh          distro package (fish 3.7.x on Ubuntu 24.04)
#   install_fish.sh 4.9.3    official static build, verified against a pinned
#                            checksum (needs curl and xz-utils)
#
# Bump a pinned version by adding its per-arch sha256 from the fish-shell
# GitHub release and updating FISH4_VERSION in docker-bake.hcl.
set -eu

version="${1:-}"

if [ -z "$version" ]; then
    apt-get update
    apt-get install -y fish
    rm -rf /var/lib/apt/lists/*
    fish --version
    exit 0
fi

arch="$(uname -m)"
case "${version}-${arch}" in
    4.9.3-x86_64) sha256=8f643d10ad1abb0ef7072764c01ceb5cf61b2d373fda400cf60f0bad69b7e095 ;;
    4.9.3-aarch64) sha256=57349f0111728c64b703a84e611aa97ff4d1fa8466385d64e67bd5bc3ffa49fc ;;
    *)
        echo "install_fish.sh: no pinned checksum for fish ${version} on ${arch}" >&2
        exit 1
        ;;
esac

tarball="fish-${version}-linux-${arch}.tar.xz"
curl -sSfL -o "/tmp/${tarball}" \
    "https://github.com/fish-shell/fish-shell/releases/download/${version}/${tarball}"
echo "${sha256}  /tmp/${tarball}" | sha256sum -c -
tar -xJf "/tmp/${tarball}" -C /usr/local/bin fish
rm -f "/tmp/${tarball}"
fish --version
