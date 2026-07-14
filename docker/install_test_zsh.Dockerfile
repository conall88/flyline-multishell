# Release-install zsh validation: runs install.sh as an end user would, then
# confirms the packaged zsh integration actually works from the installed
# locations (not from the source checkout).
FROM ubuntu:24.04@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90

# Fork/release parameters. Defaults target the current fork; override via Bake
# args to point at a different repo, version, or asset source.
ARG FLYLINE_REPO=conall88/flyline-multishell
ARG FLYLINE_INSTALL_VERSION
# When set to a local directory (e.g. /opt/flyline-assets) or an HTTP(S) base
# URL, install.sh consumes release assets from there instead of GitHub.
ARG FLYLINE_ASSET_BASE=

RUN apt-get update && apt-get install -y curl zsh && rm -rf /var/lib/apt/lists/*

# Test the current tree's installer, not a previously published copy.
COPY install.sh /tmp/flyline-install.sh
# Locally produced release assets (populated by the `build-release-assets` Bake
# target). Always present so the COPY resolves; only used when FLYLINE_ASSET_BASE
# points here.
COPY docker/build-release-assets/ /opt/flyline-assets/
COPY docker/zsh_integration_test.sh /opt/flyline/test.sh

RUN FLYLINE_REPO="${FLYLINE_REPO}" \
    FLYLINE_INSTALL_VERSION="${FLYLINE_INSTALL_VERSION}" \
    FLYLINE_ASSET_BASE="${FLYLINE_ASSET_BASE}" \
    sh /tmp/flyline-install.sh

# Validate the release-installed zsh integration:
#   1. flyline-standalone exists, is executable, and runs (--version exits cleanly).
#   2. The packaged scripts/flyline.zsh landed in the install dir.
#   3. An interactive zsh can source the script and enable/disable flyline
#      without hanging (the pty-driven test is self-bounded by timeouts).
RUN set -eux; \
    INSTALL_DIR="${HOME}/.local/lib"; \
    test -x "${INSTALL_DIR}/flyline-standalone"; \
    "${INSTALL_DIR}/flyline-standalone" --version; \
    test -f "${INSTALL_DIR}/scripts/flyline.zsh"; \
    chmod +x /opt/flyline/test.sh; \
    FLYLINE_ZSH="${INSTALL_DIR}/scripts/flyline.zsh" \
    FLYLINE_BIN="${INSTALL_DIR}/flyline-standalone" \
    /opt/flyline/test.sh

# Confirm the same installer cleanly removes both shell integrations and all
# packaged flyline files after the functional checks complete.
RUN set -eux; \
    sh /tmp/flyline-install.sh --uninstall; \
    ! grep -qF "# >>> flyline start >>>" "${HOME}/.zshrc"; \
    ! grep -q "enable -f .*libflyline.* flyline" "${HOME}/.bashrc"; \
    test ! -e "${HOME}/.local/lib/flyline-standalone"; \
    test ! -e "${HOME}/.local/lib/scripts/flyline.zsh"; \
    test ! -e "${HOME}/.local/lib/libflyline.so"; \
    test ! -e "${HOME}/.local/lib/UPSTREAM_BASE.toml"
