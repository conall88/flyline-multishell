FROM alpine:latest@sha256:28bd5fe8b56d1bd048e5babf5b10710ebe0bae67db86916198a6eec434943f8b

# Fork/release parameters. Defaults target the current fork; override via Bake
# args to point at a different repo, version, or asset source.
ARG FLYLINE_REPO=conall88/flyline-multishell
ARG FLYLINE_INSTALL_VERSION
# When set to a local directory (e.g. /opt/flyline-assets) or an HTTP(S) base
# URL, install.sh consumes release assets from there instead of GitHub. Left
# empty by default so the standard GitHub download path is exercised.
#
# NOTE: Alpine is musl. install.sh resolves the x86_64-unknown-linux-musl
# archive here, so a local FLYLINE_ASSET_BASE must contain musl-named assets
# (the `build-release-assets` target produces gnu assets by default). The
# default GitHub flow uses the musl release archive.
ARG FLYLINE_ASSET_BASE=

RUN apk add --no-cache gcc bash curl tar

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

RUN bash -i -c "flyline --version"
