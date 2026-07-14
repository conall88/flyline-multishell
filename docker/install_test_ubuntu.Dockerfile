FROM ubuntu:22.04@sha256:0e0a0fc6d18feda9db1590da249ac93e8d5abfea8f4c3c0c849ce512b5ef8982

# Fork/release parameters. Defaults target the current fork; override via Bake
# args to point at a different repo, version, or asset source.
ARG FLYLINE_REPO=conall88/flyline-multishell
ARG FLYLINE_INSTALL_VERSION
# When set to a local directory (e.g. /opt/flyline-assets) or an HTTP(S) base
# URL, install.sh consumes release assets from there instead of GitHub. Left
# empty by default so the standard GitHub download path is exercised.
ARG FLYLINE_ASSET_BASE=

RUN apt-get update && apt-get install -y curl && rm -rf /var/lib/apt/lists/*

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

RUN /bin/bash -i -c "flyline --version"

# A full uninstall removes the Bash startup line and all packaged binaries,
# libraries, scripts, and release metadata.
RUN set -eux; \
    sh /tmp/flyline-install.sh --uninstall; \
    ! grep -q "enable -f .*libflyline.* flyline" "${HOME}/.bashrc"; \
    test ! -e "${HOME}/.local/lib/libflyline.so"; \
    test ! -e "${HOME}/.local/lib/flyline-standalone"; \
    test ! -e "${HOME}/.local/lib/UPSTREAM_BASE.toml"
