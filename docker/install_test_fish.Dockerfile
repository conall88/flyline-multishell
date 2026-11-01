# Release-install fish validation: runs install.sh as an end user would, then
# confirms the packaged fish integration actually works from the installed
# locations (not from the source checkout).
FROM ubuntu:24.04@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90

# Fork/release parameters. Defaults target the current fork; override via Bake
# args to point at a different repo, version, or asset source.
ARG FLYLINE_REPO=conall88/flyline-multishell
ARG FLYLINE_INSTALL_VERSION
# When set to a local directory (e.g. /opt/flyline-assets) or an HTTP(S) base
# URL, install.sh consumes release assets from there instead of GitHub.
ARG FLYLINE_ASSET_BASE=
# Empty: the distro fish (3.7.x). Set (e.g. 4.9.3): pinned static fish 4 build.
ARG FISH_VERSION=

RUN apt-get update && apt-get install -y curl python3 xz-utils && rm -rf /var/lib/apt/lists/*
COPY docker/install_fish.sh /tmp/install_fish.sh
RUN sh /tmp/install_fish.sh "${FISH_VERSION}"

# Test the current tree's installer, not a previously published copy.
COPY install.sh /tmp/flyline-install.sh
# Locally produced release assets (populated by the `build-release-assets` Bake
# target). Always present so the COPY resolves; only used when FLYLINE_ASSET_BASE
# points here.
COPY docker/build-release-assets/ /opt/flyline-assets/
COPY docker/fish_integration_test.py /opt/flyline/test.py

RUN FLYLINE_REPO="${FLYLINE_REPO}" \
    FLYLINE_INSTALL_VERSION="${FLYLINE_INSTALL_VERSION}" \
    FLYLINE_ASSET_BASE="${FLYLINE_ASSET_BASE}" \
    sh /tmp/flyline-install.sh

# Validate the release-installed fish integration:
#   1. flyline-standalone exists, is executable, and runs (--version exits cleanly).
#   2. The packaged scripts/flyline.fish landed in the install dir.
#   3. conf.d loader was written.
#   4. An interactive fish can source the script without hanging.
RUN set -eux; \
    INSTALL_DIR="${HOME}/.local/lib"; \
    test -x "${INSTALL_DIR}/flyline-standalone"; \
    "${INSTALL_DIR}/flyline-standalone" --version; \
    test -f "${INSTALL_DIR}/scripts/flyline.fish"; \
    test -f "${HOME}/.config/fish/conf.d/flyline.fish"; \
    FLYLINE_FISH="${INSTALL_DIR}/scripts/flyline.fish" \
    FLYLINE_BIN="${INSTALL_DIR}/flyline-standalone" \
    python3 /opt/flyline/test.py

# Confirm the same installer cleanly removes fish integration and packaged files.
RUN set -eux; \
    sh /tmp/flyline-install.sh --uninstall; \
    test ! -e "${HOME}/.config/fish/conf.d/flyline.fish"; \
    test ! -e "${HOME}/.local/lib/flyline-standalone"; \
    test ! -e "${HOME}/.local/lib/scripts/flyline.fish"; \
    test ! -e "${HOME}/.local/lib/scripts/flyline.zsh"; \
    test ! -e "${HOME}/.local/lib/libflyline.so"; \
    test ! -e "${HOME}/.local/lib/UPSTREAM_BASE.toml"
