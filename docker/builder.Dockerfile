# Multi stage docker build using cargo chef.
# https://github.com/LukeMathWalker/cargo-chef
# https://lpalmieri.com/posts/fast-rust-docker-builds/
# the whole idea is to build dependencies in a separate stage and let docker cache them
# so that we don't have to recompile all dependencies on every code change.

# Stage 1: Builder - Use Ubuntu 16.04 for glibc 2.23 compatibility
# targetting this older glibc version ensures compatibility with a wide range of host systems
FROM ubuntu:16.04@sha256:1f1a2d56de1d604801a9671f301190704c25d604a416f59e03c04f5c6ffee0d6 AS chef

# Prevent interactive prompts during package installation
ENV DEBIAN_FRONTEND=noninteractive

# Install build dependencies
RUN apt-get update && apt-get install -y \
    curl \
    build-essential \
    pkg-config \
    binutils \
    && rm -rf /var/lib/apt/lists/*

# Install a pinned Rust toolchain through a checksum-verified rustup installer.
ARG RUSTUP_VERSION=1.29.0
ARG RUSTUP_INIT_SHA256_AMD64=4acc9acc76d5079515b46346a485974457b5a79893cfb01112423c89aeb5aa10
ARG RUSTUP_INIT_SHA256_ARM64=9732d6c5e2a098d3521fca8145d826ae0aaa067ef2385ead08e6feac88fa5792
ARG RUST_TOOLCHAIN=1.96.0
ARG TARGETARCH
RUN case "${TARGETARCH}" in \
        amd64) rustup_target=x86_64-unknown-linux-gnu; rustup_sha="${RUSTUP_INIT_SHA256_AMD64}" ;; \
        arm64) rustup_target=aarch64-unknown-linux-gnu; rustup_sha="${RUSTUP_INIT_SHA256_ARM64}" ;; \
        *) echo "Unsupported builder architecture: ${TARGETARCH}" >&2; exit 1 ;; \
    esac \
    && curl --proto '=https' --tlsv1.2 --fail --location --show-error --silent \
        --retry 5 \
        "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/${rustup_target}/rustup-init" \
        -o /tmp/rustup-init \
    && echo "${rustup_sha}  /tmp/rustup-init" | sha256sum -c - \
    && chmod +x /tmp/rustup-init \
    && /tmp/rustup-init -y --profile minimal --default-toolchain "${RUST_TOOLCHAIN}" \
    && rm /tmp/rustup-init
ENV PATH="/root/.cargo/bin:${PATH}"
WORKDIR /app
RUN cargo install cargo-chef --version 0.1.77 --locked


# Stage 2: Planner
FROM chef AS planner
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY examples ./examples
RUN cargo chef prepare --recipe-path recipe.json


# Stage 3: Run final Build
FROM chef AS flyline-builder
ARG CARGO_FEATURES
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release ${CARGO_FEATURES:+--features $CARGO_FEATURES} --recipe-path recipe.json
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY examples ./examples
COPY tests ./tests
RUN cargo build --release ${CARGO_FEATURES:+--features $CARGO_FEATURES} --features standalone --bin flyline-standalone \
    && cargo build --release ${CARGO_FEATURES:+--features $CARGO_FEATURES}

FROM flyline-builder AS flyline-lib-tests
ARG CARGO_FEATURES
RUN cargo test --release ${CARGO_FEATURES:+--features $CARGO_FEATURES} --lib

# CI-only stage: host-side integration tests that exercise the real built binary
# but do NOT require an interactive TTY, so a plain `docker build`/`RUN` (no `-t`,
# no allocated pty) is sufficient here:
#   * standalone_startup_tests runs flyline-standalone headless with stdin=/dev/null.
#   * zsh_completion_tests shells out to `cargo test --lib parse_capture` (pure Rust parser).
# The `standalone` feature is required so CARGO_BIN_EXE_flyline-standalone is populated;
# without it standalone_startup_tests silently skips.
# (The pty-driven zsh coverage lives in the separate zsh-integration-test bake target,
# which allocates its own pty internally via zsh/zpty.)
FROM flyline-builder AS flyline-host-integration-tests
ARG CARGO_FEATURES
RUN cargo test --release ${CARGO_FEATURES:+--features $CARGO_FEATURES} --features standalone \
        --test standalone_startup_tests \
        --test zsh_completion_tests \
        -- --nocapture


# Build image with output. This won't have anything in the file system apart from the built library
# this makes it convenient to copy the built library without creating a container
FROM scratch AS flyline-built-artifact
COPY --from=flyline-builder /app/target/release/libflyline.so /libflyline.so

# Zsh integration artifact: lib + standalone editor binary for docker/zsh_integration_test.Dockerfile.
FROM scratch AS flyline-zsh-integration-artifact
COPY --from=flyline-builder /app/target/release/libflyline.so /libflyline.so
COPY --from=flyline-builder /app/target/release/flyline-standalone /flyline-standalone
