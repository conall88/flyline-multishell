# Assembles a flyline release archive in the packaged layout from a locally
# built artifact, mirroring what the release workflow publishes. This lets the
# install-test targets exercise install.sh fully offline by pointing
# FLYLINE_ASSET_BASE at the produced directory.
#
# Expected `built-artifact` context: a stage exposing /libflyline.so and
# /flyline-standalone (docker/builder.Dockerfile `flyline-zsh-integration-artifact`).
#
# Produced archive members (top level of the tarball):
#   libflyline.so.<version_no_v>   (versioned loadable library)
#   flyline-standalone             (zsh editor binary)
#   scripts/flyline.zsh            (zsh integration script)
#   LICENSE-MIT
#   LICENSE-GPLv3
#   UPSTREAM_BASE.toml             (fork provenance metadata)
FROM alpine:latest@sha256:28bd5fe8b56d1bd048e5babf5b10710ebe0bae67db86916198a6eec434943f8b AS assembler

# Must match the version passed to the install-test target so install.sh looks
# for the same archive name and builds the correct versioned symlink.
ARG FLYLINE_INSTALL_VERSION=v0.0.0-local
# Release target the produced archive is named for. Defaults to the target the
# ubuntu:16.04-based builder produces (glibc x86_64). musl / pre_bash_4_4 assets
# require the corresponding cross/feature builds from the release workflow.
ARG RELEASE_TARGET=x86_64-unknown-linux-gnu
ARG ARCHIVE_SUFFIX=
ARG LIB_NAME=libflyline.so

RUN apk add --no-cache tar coreutils

WORKDIR /stage

COPY --from=built-artifact /libflyline.so ./lib_src
COPY --from=built-artifact /flyline-standalone ./flyline-standalone
COPY scripts/flyline.zsh ./scripts/flyline.zsh
COPY LICENSE-MIT ./LICENSE-MIT
COPY LICENSE-GPLv3 ./LICENSE-GPLv3
COPY UPSTREAM_BASE.toml ./UPSTREAM_BASE.toml

RUN set -eu; \
    case "${FLYLINE_INSTALL_VERSION}" in \
        multishell-v*) version_no_v="${FLYLINE_INSTALL_VERSION#multishell-v}" ;; \
        v*) version_no_v="${FLYLINE_INSTALL_VERSION#v}" ;; \
        *) version_no_v="${FLYLINE_INSTALL_VERSION}" ;; \
    esac; \
    lib_versioned="${LIB_NAME}.${version_no_v}"; \
    mv ./lib_src "./${lib_versioned}"; \
    chmod +x ./flyline-standalone; \
    archive="libflyline-${FLYLINE_INSTALL_VERSION}-${RELEASE_TARGET}${ARCHIVE_SUFFIX}.tar.gz"; \
    mkdir -p /out; \
    tar czf "/out/${archive}" \
        "${lib_versioned}" \
        flyline-standalone \
        scripts/flyline.zsh \
        LICENSE-MIT \
        LICENSE-GPLv3 \
        UPSTREAM_BASE.toml; \
    cd /out && sha256sum "${archive}" > "${archive}.sha256"

# Emit only the produced assets so `output = type=local` writes a clean asset dir.
FROM scratch AS assets
COPY --from=assembler /out/ /
