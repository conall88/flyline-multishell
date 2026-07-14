variable "BASH_VERSION_MATRIX" {
    default = ["4.4-rc1", "4.4.18", "5.0", "5.3"]
}

variable "PRE_BASH_4_4_VERSION_MATRIX" {
    default = ["3.2.57"]
}

variable "FLYLINE_INSTALL_VERSION" {
    default = null
}

# Release repo the installer targets fetch from. Defaults to the fork; override
# to test a different repo's published release.
variable "FLYLINE_REPO" {
    default = "conall88/flyline-multishell"
}

# Asset source override for install.sh. Empty means the standard GitHub release
# download path. The *-local install-test targets set this to the in-image asset
# directory so install.sh consumes locally produced release assets instead.
variable "FLYLINE_ASSET_BASE" {
    default = ""
}

# Version used for locally produced release assets (build-release-assets) and
# the matching *-local install-test targets. Must be identical on both so the
# archive name and versioned symlink line up.
variable "LOCAL_ASSET_VERSION" {
    default = "v0.0.0-local"
}

# Target name used for locally packaged release assets. Override this on native
# ARM hosts so the archive name matches install.sh's detected target:
#   LOCAL_ASSET_TARGET=aarch64-unknown-linux-gnu docker buildx bake ...
variable "LOCAL_ASSET_TARGET" {
    default = "x86_64-unknown-linux-gnu"
}

# In-image directory the install-test Dockerfiles copy local assets into.
variable "LOCAL_ASSET_DIR" {
    default = "/opt/flyline-assets"
}

target "builder" {
    context = "."
    dockerfile = "docker/builder.Dockerfile"
    target = "flyline-builder"
}

target "built-artifact" {
    context = "."
    dockerfile = "docker/builder.Dockerfile"
    target = "flyline-built-artifact"
}

# Builder stage exposing both the loadable library and the standalone editor
# binary. Used as an input context for building local release assets.
target "zsh-integration-artifact" {
    context = "."
    dockerfile = "docker/builder.Dockerfile"
    target = "flyline-zsh-integration-artifact"
}

# Assembles a packaged release archive (versioned lib + flyline-standalone +
# scripts/flyline.zsh + licenses + UPSTREAM_BASE.toml) plus its .sha256 from a
# locally built artifact, writing them to docker/build-release-assets so the
# *-local install-test targets can consume them via FLYLINE_ASSET_BASE.
#
# example:
#   docker buildx bake -f docker-bake.hcl build-release-assets
#   docker buildx bake -f docker-bake.hcl install-test-ubuntu-local
target "build-release-assets" {
    context = "."
    contexts = {
        built-artifact = "target:zsh-integration-artifact"
    }
    dockerfile = "docker/release_assets.Dockerfile"
    target = "assets"
    output = ["type=local,dest=docker/build-release-assets"]
    args = {
        FLYLINE_INSTALL_VERSION = LOCAL_ASSET_VERSION
        RELEASE_TARGET = LOCAL_ASSET_TARGET
    }
}

# example command:
# docker buildx bake -f docker-bake.hcl extract-release-artifact
target "extract-release-artifact" {
    context = "."
    output = ["type=local,dest=docker/build"]
    dockerfile = "docker/builder.Dockerfile"
    target = "flyline-built-artifact"
}

target "extract-integration-test-build-artifact" {
    context = "."
    output = ["type=local,dest=docker/build-integration-test"]
    dockerfile = "docker/builder.Dockerfile"
    target = "flyline-built-artifact"
}

target "extract-pre-bash-4-4-integration-test-build-artifact" {
    context = "."
    platforms = ["linux/amd64"]
    output = ["type=local,dest=docker/build-pre-bash-4-4-integration-test"]
    dockerfile = "docker/builder.Dockerfile"
    target = "flyline-built-artifact"
    args = {
        CARGO_FEATURES = "pre_bash_4_4"
    }
}

target "lib-tests" {
    context = "."
    dockerfile = "docker/builder.Dockerfile"
    target = "flyline-lib-tests"
}

target "specific-bash-version" {
    context = "."
    dockerfile = "docker/specific_bash_version.Dockerfile"
    name = "specific-bash-version-${replace(docker_bash_version, ".", "_")}"
    matrix = {
        docker_bash_version = BASH_VERSION_MATRIX
    }
    args = {
        DOCKER_BASH_VERSION = docker_bash_version
    }
    tags = ["bash-${docker_bash_version}"]
}

target "specific-bash-version-pre-4-4" {
    context = "."
    dockerfile = "docker/specific_bash_version.Dockerfile"
    # Bash 3.2's bundled config.guess predates aarch64. This release variant is
    # only published for x86_64 GNU Linux, so build and test it on that platform.
    platforms = ["linux/amd64"]
    name = "specific-bash-version-${replace(docker_bash_version, ".", "_")}"
    matrix = {
        docker_bash_version = PRE_BASH_4_4_VERSION_MATRIX
    }
    args = {
        DOCKER_BASH_VERSION = docker_bash_version
    }
    tags = ["bash-${docker_bash_version}"]
}

target "bash-integration-tests" {
    context = "."
    contexts = {
        built-artifact = "target:extract-integration-test-build-artifact",
        specific-bash-version = "target:specific-bash-version-${replace(docker_bash_version, ".", "_")}"
    }
    name = "bash-integration-test-${replace(docker_bash_version, ".", "_")}"
    matrix = {
        docker_bash_version = BASH_VERSION_MATRIX
    }
    dockerfile = "docker/bash_integration_test.Dockerfile"
    args = {
        DOCKER_BASH_VERSION = docker_bash_version
    }
}

target "bash-integration-tests-pre-4-4" {
    context = "."
    platforms = ["linux/amd64"]
    contexts = {
        built-artifact = "target:extract-pre-bash-4-4-integration-test-build-artifact",
        specific-bash-version = "target:specific-bash-version-${replace(docker_bash_version, ".", "_")}"
    }
    name = "bash-integration-test-${replace(docker_bash_version, ".", "_")}"
    tags = ["bash-integration-test-pre-4-4-${docker_bash_version}"]
    matrix = {
        docker_bash_version = PRE_BASH_4_4_VERSION_MATRIX
    }
    dockerfile = "docker/bash_integration_test.Dockerfile"
    args = {
        DOCKER_BASH_VERSION = docker_bash_version
    }
}

target "extract-zsh-integration-test-build-artifact" {
    context = "."
    output = ["type=local,dest=docker/build-zsh-integration-test"]
    dockerfile = "docker/builder.Dockerfile"
    target = "flyline-zsh-integration-artifact"
}

target "zsh-integration-test" {
    context = "."
    contexts = {
        built-artifact = "target:extract-zsh-integration-test-build-artifact"
    }
    dockerfile = "docker/zsh_integration_test.Dockerfile"
}




# Runs `flyline --help` inside an interactive bash session, strips ANSI codes,
# and outputs flyline_help.txt to the project root.
target "extract-help-text" {
    context = "."
    contexts = {
        built-artifact = "target:built-artifact"
    }
    output = ["type=local,dest=./"]
    dockerfile = "docker/flyline_help.Dockerfile"
    target = "flyline-help-output"
}


target "demo-base" {
    context = "."
    dockerfile = "docker/demo_base.Dockerfile"
    contexts = {
        flyline-extracted-library = "target:built-artifact"
    }
}

target "_demo-base" {
    context = "."
    contexts = {
        demo-base = "target:demo-base"
    }
    output = ["type=local,dest=./"]
    # Sets the hostname for the build sandbox; used by \h in the PS1 prompt during VHS recording.
    args = {
        BUILDKIT_SANDBOX_HOSTNAME = "my-hostname"
    }
}


target "demo-overview-extracted" {
    inherits = ["_demo-base"]
    dockerfile = "docker/demo_overview.Dockerfile"
}

target "demo-prompts-extracted" {
    inherits = ["_demo-base"]
    dockerfile = "docker/demo_prompts.Dockerfile"
}

target "demo-fuzzy-suggestions-extracted" {
    inherits = ["_demo-base"]
    dockerfile = "docker/demo_fuzzy_suggestions.Dockerfile"
}

target "demo-fuzzy-path-suggestions-extracted" {
    inherits = ["_demo-base"]
    dockerfile = "docker/demo_fuzzy_path_suggestions.Dockerfile"
}

target "demo-custom-animation-extracted" {
    inherits = ["_demo-base"]
    dockerfile = "docker/demo_custom_animation.Dockerfile"
}

target "demo-agent-mode-extracted" {
    inherits = ["_demo-base"]
    dockerfile = "docker/demo_agent_mode.Dockerfile"
}

target "demo-ls-colors-extracted" {
    inherits = ["_demo-base"]
    dockerfile = "docker/demo_ls_colors.Dockerfile"
}

target "demo-fuzzy-history-extracted" {
    inherits = ["_demo-base"]
    dockerfile = "docker/demo_fuzzy_history.Dockerfile"
}

target "demo-inline-history-extracted" {
    inherits = ["_demo-base"]
    dockerfile = "docker/demo_inline_history.Dockerfile"
}

target "demo-tab-completion-easing-extracted" {
    inherits = ["_demo-base"]
    dockerfile = "docker/demo_tab_completion_easing.Dockerfile"
}

target "demo-auto-tab-completion-extracted" {
    inherits = ["_demo-base"]
    dockerfile = "docker/demo_auto_tab_completion.Dockerfile"
}

target "demo-flycomp-extracted" {
    inherits = ["_demo-base"]
    dockerfile = "docker/demo_flycomp.Dockerfile"
}

target "demo-cursor-style-extracted" {
    inherits = ["_demo-base"]
    dockerfile = "docker/demo_cursor_style.Dockerfile"
}

group "demos" {
    targets = [
        "demo-overview-extracted",
        "demo-prompts-extracted",
        "demo-fuzzy-suggestions-extracted",
        "demo-fuzzy-path-suggestions-extracted",
        "demo-custom-animation-extracted",
        "demo-agent-mode-extracted",
        "demo-ls-colors-extracted",
        "demo-fuzzy-history-extracted",
        "demo-inline-history-extracted",
        "demo-tab-completion-easing-extracted",
        "demo-auto-tab-completion-extracted",
        "demo-flycomp-extracted",
        "demo-cursor-style-extracted"
    ]
}

# ---------------------------------------------------------------------------
# Installer tests
# ---------------------------------------------------------------------------
#
# Base targets consume either published assets or the complete locally served
# asset set assembled by release.yml. The GNU-compatible local variants below
# consume docker/build-release-assets (run `build-release-assets` first).

target "install-test-alpine" {
    context = "."
    dockerfile = "docker/install_test_alpine.Dockerfile"
    args = {
        FLYLINE_REPO = FLYLINE_REPO
        FLYLINE_INSTALL_VERSION = FLYLINE_INSTALL_VERSION
        FLYLINE_ASSET_BASE = FLYLINE_ASSET_BASE
    }
}

target "install-test-ubuntu" {
    context = "."
    dockerfile = "docker/install_test_ubuntu.Dockerfile"
    args = {
        FLYLINE_REPO = FLYLINE_REPO
        FLYLINE_INSTALL_VERSION = FLYLINE_INSTALL_VERSION
        FLYLINE_ASSET_BASE = FLYLINE_ASSET_BASE
    }
}

target "install-test-arch" {
    context = "."
    dockerfile = "docker/install_test_arch.Dockerfile"
    args = {
        FLYLINE_REPO = FLYLINE_REPO
        FLYLINE_INSTALL_VERSION = FLYLINE_INSTALL_VERSION
        FLYLINE_ASSET_BASE = FLYLINE_ASSET_BASE
    }
}


target "install-test-bash-3-2-57" {
    context = "."
    platforms = ["linux/amd64"]
    contexts = {
        specific-bash-version = "target:specific-bash-version-3_2_57"
    }
    dockerfile = "docker/install_test_bash_3.2.57.Dockerfile"
    args = {
        FLYLINE_REPO = FLYLINE_REPO
        FLYLINE_INSTALL_VERSION = FLYLINE_INSTALL_VERSION
        FLYLINE_ASSET_BASE = FLYLINE_ASSET_BASE
    }
}

# Release-install zsh validation (GitHub-release flow): installs via install.sh,
# then confirms flyline-standalone runs, the packaged scripts/flyline.zsh is
# present, and an interactive zsh can source/enable/disable without hanging.
target "install-test-release-zsh" {
    context = "."
    dockerfile = "docker/install_test_zsh.Dockerfile"
    args = {
        FLYLINE_REPO = FLYLINE_REPO
        FLYLINE_INSTALL_VERSION = FLYLINE_INSTALL_VERSION
        FLYLINE_ASSET_BASE = FLYLINE_ASSET_BASE
    }
}

# ---- Local-asset variants (consume docker/build-release-assets) ----

target "install-test-ubuntu-local" {
    inherits = ["install-test-ubuntu"]
    args = {
        FLYLINE_INSTALL_VERSION = LOCAL_ASSET_VERSION
        FLYLINE_ASSET_BASE = LOCAL_ASSET_DIR
    }
}

target "install-test-arch-local" {
    inherits = ["install-test-arch"]
    args = {
        FLYLINE_INSTALL_VERSION = LOCAL_ASSET_VERSION
        FLYLINE_ASSET_BASE = LOCAL_ASSET_DIR
    }
}

target "install-test-release-zsh-local" {
    inherits = ["install-test-release-zsh"]
    args = {
        FLYLINE_INSTALL_VERSION = LOCAL_ASSET_VERSION
        FLYLINE_ASSET_BASE = LOCAL_ASSET_DIR
    }
}

group "install-tests" {
    targets = [
        "install-test-alpine",
        "install-test-ubuntu",
        "install-test-arch",
        "install-test-bash-3-2-57",
        "install-test-release-zsh",
    ]
}

# Local end-to-end GNU installer tests against locally produced assets. The
# musl and pre-Bash variants are exercised by release.yml using genuine cross-
# compiled artifacts rather than relabeling this native GNU build.
group "install-tests-local" {
    targets = [
        "install-test-ubuntu-local",
        "install-test-arch-local",
        "install-test-release-zsh-local",
    ]
}

# Native ARM hosts cannot run the official Arch Linux image (it is x86_64-only).
# This group exercises the portable GNU/Linux and zsh release-install paths on
# Apple Silicon and GitHub's ubuntu-24.04-arm runners.
group "install-tests-local-arm64" {
    targets = [
        "install-test-ubuntu-local",
        "install-test-release-zsh-local",
    ]
}

