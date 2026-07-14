FROM archlinux:latest@sha256:681569955d1d17313ef7134acc8b5cd8adcda2fc24709bed472d95e1cf3d71a1

# Fork/release parameters. Defaults target the current fork; override via Bake
# args to point at a different repo, version, or asset source.
ARG FLYLINE_REPO=conall88/flyline-multishell
ARG FLYLINE_INSTALL_VERSION
# When set to a local directory (e.g. /opt/flyline-assets) or an HTTP(S) base
# URL, install.sh consumes release assets from there instead of GitHub. Left
# empty by default so the standard GitHub download path is exercised.
ARG FLYLINE_ASSET_BASE=

RUN pacman -Syu --noconfirm && \
    pacman -S --noconfirm curl bash tar

# Test the current tree's installer, not a previously published copy.
COPY install.sh /tmp/flyline-install.sh
# Locally produced release assets (populated by the `build-release-assets` Bake
# target). Always present so the COPY resolves; only used when FLYLINE_ASSET_BASE
# points here.
COPY docker/build-release-assets/ /opt/flyline-assets/

RUN FLYLINE_REPO="${FLYLINE_REPO}" \
    FLYLINE_INSTALL_VERSION="${FLYLINE_INSTALL_VERSION}" \
    FLYLINE_ASSET_BASE="${FLYLINE_ASSET_BASE}" \
    sh /tmp/flyline-install.sh

# Run bash interactively to load flyline and test that it doesn't crash/fail on Arch Linux's strict dynamic linking environment
RUN /bin/bash -i -c "flyline --version"
