#!/bin/sh
# Flyline installer
# Usage: curl -sSfL https://github.com/conall88/flyline-multishell/releases/latest/download/install.sh | sh
#        curl -sSfL …/releases/latest/download/install.sh | FLYLINE_CHANNEL=dev sh
#        sh install.sh --uninstall

set -eu

expand_path() {
    # The quoted tilde is a literal config-path prefix, not shell expansion.
    # shellcheck disable=SC2088
    case "$1" in
        '~/'*) echo "${HOME}/${1#~/}" ;;
        '~')   echo "${HOME}" ;;
        *)     echo "$1" ;;
    esac
}

# Default release repo. Override with FLYLINE_REPO to install from a fork.
REPO="${FLYLINE_REPO:-conall88/flyline-multishell}"
# Optional asset source override. When set, release archives (and their
# .sha256 files) are fetched from here instead of the GitHub release. It may be
# an HTTP(S) base URL (e.g. http://localhost:8000/assets) or a local directory /
# file:// base (e.g. /opt/flyline-assets or file:///opt/flyline-assets). This is
# used by the Docker install tests to consume locally produced release assets;
# normal GitHub-based installs leave it unset.
FLYLINE_ASSET_BASE="${FLYLINE_ASSET_BASE:-}"
if [ -n "${FLYLINE_INSTALL_DIR:-}" ]; then
    INSTALL_DIR="$(expand_path "$FLYLINE_INSTALL_DIR")"
elif [ -n "${FLYLINE_LOAD_DIR:-}" ]; then
    INSTALL_DIR="$(expand_path "$FLYLINE_LOAD_DIR")"
else
    INSTALL_DIR="${HOME}/.local/lib"
fi
BASHRC="${HOME}/.bashrc"
ZSHRC="${HOME}/.zshrc"
FLYLINE_BASHRC_MARKER="# Flyline - enhanced Bash experience"
FLYLINE_ZSHRC_START="# >>> flyline start >>>"
FLYLINE_ZSHRC_END="# <<< flyline end <<<"
STANDALONE_BIN="flyline-standalone"
FISH_CONFD="${XDG_CONFIG_HOME:-${HOME}/.config}/fish/conf.d/flyline.fish"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

say() { printf '\033[1;34m==> \033[0m%s\n' "$*"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }
err() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }
err_no_exit() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; }

need_cmd() {
    command -v "$1" >/dev/null 2>&1 || err "Required command not found: $1"
}

# Drop the download staging area plus any half-copied files this run left in the
# install directory, so an aborted install does not litter it.
cleanup_install_tmp() {
    if [ -n "${TMP_DIR:-}" ]; then
        rm -rf "$TMP_DIR"
    fi
    if [ -n "${INSTALL_DIR:-}" ]; then
        rm -f "${INSTALL_DIR}/.flyline-tmp.$$".* "${INSTALL_DIR}/scripts/.flyline-tmp.$$".*
    fi
    return 0
}

# Copy one file into place through a temp name in the destination directory, then
# rename it. A rename cannot be observed or interrupted half-done, so a failed
# install can never leave a partially written file where an existing
# libflyline.<ext> symlink already points. That matters because a truncated
# loadable library does not merely fail to load: Bash aborts with a bus error
# while running ~/.bashrc, so every new terminal dies.
install_file_atomic() {
    src="$1"
    dst="$2"
    tmp="${dst%/*}/.flyline-tmp.$$.${dst##*/}"
    cp "$src" "$tmp"
    if [ -x "$src" ]; then
        chmod +x "$tmp"
    fi
    mv -f "$tmp" "$dst"
}

# Install the unpacked archive tree into the destination, one atomic file at a
# time. The body is a subshell so the recursive call cannot clobber the caller's
# loop state.
install_staged_tree() (
    stage_dir="$1"
    dest_dir="$2"
    mkdir -p "$dest_dir"
    for src in "$stage_dir"/* "$stage_dir"/.[!.]*; do
        [ -e "$src" ] || continue
        base="${src##*/}"
        if [ -d "$src" ]; then
            install_staged_tree "$src" "${dest_dir}/${base}"
        else
            install_file_atomic "$src" "${dest_dir}/${base}"
        fi
    done
)

download() {
    url="$1"
    dest="$2"
    if command -v curl >/dev/null 2>&1; then
        curl -sSfL --retry 5 --retry-delay 2 --retry-connrefused -o "$dest" "$url"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "$dest" "$url"
    else
        err "Neither curl nor wget is available. Please install one and retry."
    fi
}

# Fetch a named release asset into a destination path. Honors FLYLINE_ASSET_BASE
# when set (HTTP(S) base URL, or local directory / file:// base), otherwise falls
# back to the standard GitHub release download URL. Keeps normal GitHub behavior
# unchanged for end users (FLYLINE_ASSET_BASE unset).
fetch_asset() {
    asset_name="$1"
    dest="$2"
    if [ -n "$FLYLINE_ASSET_BASE" ]; then
        base="${FLYLINE_ASSET_BASE%/}"
        src="${base}/${asset_name}"
        case "$src" in
            http://* | https://*)
                download "$src" "$dest"
                ;;
            file://*)
                path="${src#file://}"
                [ -f "$path" ] || err "Asset not found at ${path} (FLYLINE_ASSET_BASE=${FLYLINE_ASSET_BASE})."
                cp "$path" "$dest"
                ;;
            *)
                [ -f "$src" ] || err "Asset not found at ${src} (FLYLINE_ASSET_BASE=${FLYLINE_ASSET_BASE})."
                cp "$src" "$dest"
                ;;
        esac
    else
        download "https://github.com/${REPO}/releases/download/${VERSION}/${asset_name}" "$dest"
    fi
}

get_latest_version() {
    url="https://github.com/${REPO}/releases/latest"
    if command -v curl >/dev/null 2>&1; then
        tag_url="$(curl -sI "$url" | grep -i '^location:' | head -1)"
    elif command -v wget >/dev/null 2>&1; then
        tag_url="$(wget --max-redirect=0 --server-response -O /dev/null "$url" 2>&1 | grep -i 'location:' | head -1)"
    else
        err "Neither curl nor wget is available. Please install one and retry."
    fi
    version="$(printf '%s' "$tag_url" | sed 's|.*/||' | cut -d' ' -f1 | tr -d '\r\n')"
    [ -n "$version" ] || err "Could not determine latest version from GitHub Release redirect."
    echo "$version"
}

# Pick the newest published GitHub release tag for a channel from one page of
# releases JSON (stdin). python3 is only needed for FLYLINE_CHANNEL=dev|prerelease.
# Exit codes: 0 printed a tag, 1 no match on this page, 2 the payload is not a
# release list (API error object, rate-limit body, invalid JSON), 3 the page is
# empty so the release list is exhausted.
pick_release_tag_from_json() {
    channel="$1"
    python3 -c '
import json, sys
channel = sys.argv[1]
try:
    releases = json.load(sys.stdin)
except ValueError:
    raise SystemExit(2)
if not isinstance(releases, list):
    raise SystemExit(2)
if not releases:
    raise SystemExit(3)
if channel == "dev":
    prefix = "dev-"
elif channel == "prerelease":
    prefix = "multishell-v"
else:
    raise SystemExit(2)
for release in releases:
    if not isinstance(release, dict) or release.get("draft"):
        continue
    tag = release.get("tag_name") or ""
    if tag.startswith(prefix) and bool(release.get("prerelease")):
        print(tag)
        raise SystemExit(0)
raise SystemExit(1)
' "$channel"
}

# Fetch one page of the releases API. Sends GITHUB_TOKEN/GH_TOKEN when set:
# unauthenticated requests are capped at 60/hour per IP, which a CI runner or a
# shared NAT can exhaust.
fetch_releases_page() {
    page="$1"
    url="https://api.github.com/repos/${REPO}/releases?per_page=100&page=${page}"
    token="${GITHUB_TOKEN:-${GH_TOKEN:-}}"
    if command -v curl >/dev/null 2>&1; then
        if [ -n "$token" ]; then
            curl -sSfL --retry 5 --retry-delay 2 --retry-connrefused \
                -H 'Accept: application/vnd.github+json' \
                -H "Authorization: Bearer ${token}" "$url"
        else
            curl -sSfL --retry 5 --retry-delay 2 --retry-connrefused \
                -H 'Accept: application/vnd.github+json' "$url"
        fi
    elif command -v wget >/dev/null 2>&1; then
        if [ -n "$token" ]; then
            wget -qO- --header='Accept: application/vnd.github+json' \
                --header="Authorization: Bearer ${token}" "$url"
        else
            wget -qO- --header='Accept: application/vnd.github+json' "$url"
        fi
    else
        err "Neither curl nor wget is available. Please install one and retry."
    fi
}

# Resolve the release tag to install for a channel:
#   stable      GitHub's releases/latest redirect (skips prereleases)
#   prerelease  newest published multishell-v* prerelease
#   dev         newest published dev-* snapshot
get_channel_version() {
    channel="$1"
    case "$channel" in
        stable)
            get_latest_version
            return
            ;;
        dev | prerelease) : ;;
        *)
            err "Unknown FLYLINE_CHANNEL '${channel}'. Use stable, prerelease, or dev."
            ;;
    esac

    command -v python3 >/dev/null 2>&1 \
        || err "FLYLINE_CHANNEL=${channel} needs python3, or set FLYLINE_INSTALL_VERSION to a tag."

    # Dev snapshots and product releases share one release list, so the newest
    # release for a channel can sit past the first page once snapshots pile up.
    page=1
    page_limit=10
    while :; do
        json="$(fetch_releases_page "$page")" \
            || err "Could not read the GitHub releases API for ${REPO}. Unauthenticated requests are capped at 60/hour per IP: set GITHUB_TOKEN, or pin FLYLINE_INSTALL_VERSION=<tag>."
        status=0
        tag="$(printf '%s' "$json" | pick_release_tag_from_json "$channel")" || status=$?
        case "$status" in
            0)
                echo "$tag"
                return 0
                ;;
            2)
                err "Unexpected response from the GitHub releases API for ${REPO} (rate limited, or not a release list). Set GITHUB_TOKEN, or pin FLYLINE_INSTALL_VERSION=<tag>."
                ;;
            3)
                break
                ;;
        esac
        if [ "$page" -ge "$page_limit" ]; then
            err "No published ${channel} release for ${REPO} in the newest $((page_limit * 100)) releases. Pin FLYLINE_INSTALL_VERSION=<tag> instead."
        fi
        page=$((page + 1))
    done
    err "No published ${channel} release found for ${REPO}."
}

# Version suffix of the packaged library (libflyline.so.<cargo version>).
# Product tags encode that in the git tag; dev tags do not, so for `dev-*` the
# suffix is read back from the unpacked archive. Callers must pass the staging
# directory, never the install destination: a leftover versioned library there
# can sort ahead of the new one under POSIX glob and win. Exactly one versioned
# match is required.
lib_version_suffix() {
    version="$1"
    search_dir="$2"
    lib_name="$3"
    case "$version" in
        multishell-v*) echo "${version#multishell-v}" ;;
        v*)            echo "${version#v}" ;;
        dev-*)
            found=""
            for path in "${search_dir}/${lib_name}".[0-9]*; do
                [ -f "$path" ] || continue
                if [ -n "$found" ]; then
                    return 1
                fi
                found="${path##*"${lib_name}".}"
            done
            [ -n "$found" ] || return 1
            echo "$found"
            ;;
        *) echo "$version" ;;
    esac
}

# ---------------------------------------------------------------------------
# Platform detection
# ---------------------------------------------------------------------------

# Detect the version of the system bash as "major minor" integers.
detect_bash_version_parts() {
    bash_bin="$(command -v bash 2>/dev/null || true)"
    [ -n "$bash_bin" ] || { echo "0 0"; return; }
    # Expanded by the Bash subprocess, not by this POSIX shell.
    # shellcheck disable=SC2016
    "$bash_bin" -c 'echo "${BASH_VERSINFO[0]} ${BASH_VERSINFO[1]}"' 2>/dev/null || echo "0 0"
}

# Returns 0 (true) if the given major.minor version is >= 4.4, 1 (false) otherwise.
is_bash_version_4_4_or_later() {
    major="$1"; minor="$2"
    [ "${major:-0}" -gt 4 ] || { [ "${major:-0}" -eq 4 ] && [ "${minor:-0}" -ge 4 ]; }
}

# Returns 0 (true) if the system bash is older than 4.4, 1 (false) otherwise.
is_system_bash_pre_4_4() {
    version_str="$(detect_bash_version_parts)"
    major="${version_str%% *}"
    minor="${version_str##* }"
    ! is_bash_version_4_4_or_later "$major" "$minor"
}

# Returns the path to a Homebrew-installed bash >= 4.4, or an empty string.
find_homebrew_bash() {
    for candidate in "/opt/homebrew/bin/bash" "/usr/local/bin/bash"; do
        if [ -x "$candidate" ]; then
            # Expanded by the Bash subprocess, not by this POSIX shell.
            # shellcheck disable=SC2016
            v="$("$candidate" -c 'echo "${BASH_VERSINFO[0]} ${BASH_VERSINFO[1]}"' 2>/dev/null || echo "0 0")"
            major="${v%% *}"; minor="${v##* }"
            if is_bash_version_4_4_or_later "$major" "$minor"; then
                echo "$candidate"
                return
            fi
        fi
    done
    echo ""
}

detect_os() {
    if [ -n "${TERMUX_VERSION:-}" ] || [ -d "/data/data/com.termux" ] || (uname -o 2>/dev/null | grep -qi android) || (uname -a 2>/dev/null | grep -qi android); then
        echo "android"
        return
    fi
    os="$(uname -s)"
    case "$os" in
        Linux) echo "linux" ;;
        Darwin) echo "darwin" ;;
        FreeBSD) echo "freebsd" ;;
        *) err "Unsupported OS: $os" ;;
    esac
}

detect_arch() {
    arch="$(uname -m)"
    case "$arch" in
        x86_64 | amd64) echo "x86_64" ;;
        aarch64 | arm64) echo "aarch64" ;;
        armv7* | armhf) echo "armv7" ;;
        i386 | i486 | i586 | i686) echo "i686" ;;
        riscv64) echo "riscv64gc" ;;
        ppc64le | powerpc64le) echo "powerpc64le" ;;
        *) err "Unsupported architecture: $arch" ;;
    esac
}

detect_libc() {
    # 1. Inspect the interpreter of the running shell executable — most reliable.
    shell_exe="/proc/$$/exe"
    if [ ! -e "$shell_exe" ]; then
        shell_exe="$(command -v sh || true)"
    fi
    if [ -n "$shell_exe" ] && command -v readelf >/dev/null 2>&1; then
        interp="$(readelf -l "$shell_exe" 2>/dev/null | grep 'interpreter' | grep -o '\[.*\]' | tr -d '[]')" || true
        case "$interp" in
            *musl*) echo "musl"; return ;;
            *) echo "gnu"; return ;;
        esac
    fi

    # 2. Ask ldd directly — musl's ldd prints "musl libc" on --version.
    if ldd --version 2>&1 | grep -qi musl; then
        echo "musl"
        return
    fi

    # 3. Look for the musl dynamic linker on disk.
    if ls /lib/ld-musl-* >/dev/null 2>&1; then
        echo "musl"
        return
    fi

    # 4. Fall back to GNU libc.
    echo "gnu"
}

# ---------------------------------------------------------------------------
# Release target support
# ---------------------------------------------------------------------------
#
# Only the targets below have release archives built (see the release
# workflow's build matrix). We reject unsupported combinations up front rather
# than constructing download URLs that will 404.

# Returns 0 (true) if a standard (non pre-Bash-4.4) archive is built for TARGET.
is_supported_target() {
    case "$1" in
        x86_64-unknown-linux-gnu | \
        x86_64-unknown-linux-musl | \
        aarch64-unknown-linux-gnu | \
        aarch64-unknown-linux-musl | \
        armv7-unknown-linux-gnueabihf | \
        i686-unknown-linux-gnu | \
        riscv64gc-unknown-linux-gnu | \
        powerpc64le-unknown-linux-gnu | \
        x86_64-unknown-freebsd | \
        x86_64-apple-darwin | \
        aarch64-apple-darwin)
            return 0
            ;;
        *)
            return 1
            ;;
    esac
}

# Returns 0 (true) if a pre-Bash-4.4 archive is built for TARGET. Only the
# x86_64 GNU Linux target ships a _pre_bash_4_4 build.
is_supported_pre_bash_4_4_target() {
    [ "$1" = "x86_64-unknown-linux-gnu" ]
}

# ---------------------------------------------------------------------------

# ---------------------------------------------------------------------------
# Helpers for portability
# ---------------------------------------------------------------------------

# Portable checksum verification: supports sha256sum (Linux) and shasum (macOS).
verify_sha256() {
    sha256_file="$1"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum -c "$sha256_file"
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 -c "$sha256_file"
    else
        err "No checksum tool found (sha256sum or shasum). Cannot verify download."
    fi
}

# ---------------------------------------------------------------------------
# Zsh integration
# ---------------------------------------------------------------------------

has_zsh() {
    command -v zsh >/dev/null 2>&1
}

zshrc_has_flyline_block() {
    [ -f "$ZSHRC" ] && grep -qF "$FLYLINE_ZSHRC_START" "$ZSHRC"
}

backup_zshrc_if_needed() {
    if [ ! -f "$ZSHRC" ]; then
        return
    fi
    if zshrc_has_flyline_block; then
        return
    fi
    ts="$(date +%Y%m%d%H%M%S)"
    cp "$ZSHRC" "${ZSHRC}.flyline.bak.${ts}"
    say "Backed up ${ZSHRC} to ${ZSHRC}.flyline.bak.${ts}"
}

# Release installs ship scripts/flyline.zsh inside the archive, so it is already
# present under INSTALL_DIR/scripts after extraction. Consume that packaged copy
# directly (no unchecked raw.githubusercontent fallback). The --local flow links
# the checkout's script separately in local_main().
install_flyline_zsh_script() {
    dest="${INSTALL_DIR}/scripts/flyline.zsh"
    if [ -f "$dest" ]; then
        return
    fi
    err "Packaged scripts/flyline.zsh not found in ${INSTALL_DIR}/scripts. The release archive is expected to contain scripts/flyline.zsh."
}

install_zsh_integration() {
    if ! has_zsh; then
        return
    fi

    install_flyline_zsh_script

    standalone_path="${INSTALL_DIR}/${STANDALONE_BIN}"
    if [ -f "$standalone_path" ]; then
        chmod +x "$standalone_path"
        say "Installed zsh editor: ${standalone_path}"
    else
        warn "zsh detected but ${standalone_path} is not installed yet."
        warn "Zsh integration will stay disabled until the standalone binary is available."
    fi

    ensure_zshrc_block
}

has_fish() {
    command -v fish >/dev/null 2>&1
}

# Release installs ship scripts/flyline.fish inside the archive, so it is already
# under INSTALL_DIR/scripts after tar extraction. Missing means a bad archive.
install_flyline_fish_script() {
    dest="${INSTALL_DIR}/scripts/flyline.fish"
    if [ -f "$dest" ]; then
        return
    fi
    err "Packaged scripts/flyline.fish not found in ${INSTALL_DIR}/scripts. The release archive is expected to contain scripts/flyline.fish."
}

# Write the conf.d loader (fish auto-sources conf.d — no config.fish edits, so
# no backup is needed; the file is entirely flyline-owned and safe to overwrite).
write_fish_confd() {
    mkdir -p "$(dirname "$FISH_CONFD")"
    cat > "$FISH_CONFD" <<EOF
# >>> flyline start >>>
set -gx FLYLINE_BIN "${INSTALL_DIR}/${STANDALONE_BIN}"
test -r "${INSTALL_DIR}/scripts/flyline.fish"; and source "${INSTALL_DIR}/scripts/flyline.fish"
# <<< flyline end <<<
EOF
    say "Wrote fish loader: ${FISH_CONFD}"
}

install_fish_integration() {
    if ! has_fish; then
        return
    fi

    install_flyline_fish_script

    standalone_path="${INSTALL_DIR}/${STANDALONE_BIN}"
    if [ -f "$standalone_path" ]; then
        chmod +x "$standalone_path"
        say "Installed fish editor: ${standalone_path}"
    else
        warn "fish detected but ${standalone_path} is not installed yet."
        warn "Fish integration will stay disabled until the standalone binary is available."
    fi

    write_fish_confd
}

remove_fish_integration() {
    rm -f "$FISH_CONFD"
    say "Removed fish conf.d loader (if present)"
}

# Append the guarded flyline block to ~/.zshrc (idempotent; backs up first).
ensure_zshrc_block() {
    if zshrc_has_flyline_block; then
        say "Flyline zsh block already present in ${ZSHRC}; skipping."
        return
    fi

    backup_zshrc_if_needed
    touch "$ZSHRC"
    # shellcheck disable=SC2016
    cat >> "$ZSHRC" <<EOF

${FLYLINE_ZSHRC_START}
export FLYLINE_BIN="${INSTALL_DIR}/${STANDALONE_BIN}"
[[ -r "${INSTALL_DIR}/scripts/flyline.zsh" ]] && . "${INSTALL_DIR}/scripts/flyline.zsh"
${FLYLINE_ZSHRC_END}
EOF
    say "Added flyline zsh block to ${ZSHRC}"
}

remove_zshrc_flyline_block() {
    if [ ! -f "$ZSHRC" ]; then
        return
    fi
    if ! grep -qF "$FLYLINE_ZSHRC_START" "$ZSHRC"; then
        return
    fi
    tmp="$(mktemp "${TMPDIR:-/tmp}/flyline.zshrc.XXXXXX")"
    awk -v start="$FLYLINE_ZSHRC_START" -v end="$FLYLINE_ZSHRC_END" '
        $0 == start { skip = 1; next }
        $0 == end   { skip = 0; next }
        !skip { print }
    ' "$ZSHRC" > "$tmp"
    mv "$tmp" "$ZSHRC"
    say "Removed flyline block from ${ZSHRC}"
}

remove_bashrc_flyline_lines() {
    if [ ! -f "$BASHRC" ]; then
        return
    fi

    bash_enable_so="enable -f ${INSTALL_DIR}/libflyline.so flyline"
    bash_enable_dylib="enable -f ${INSTALL_DIR}/libflyline.dylib flyline"
    if ! grep -qF "$FLYLINE_BASHRC_MARKER" "$BASHRC" \
        && ! grep -qF "$bash_enable_so" "$BASHRC" \
        && ! grep -qF "$bash_enable_dylib" "$BASHRC"; then
        return
    fi

    tmp="$(mktemp "${TMPDIR:-/tmp}/flyline.bashrc.XXXXXX")"
    awk \
        -v marker="$FLYLINE_BASHRC_MARKER" \
        -v enable_so="$bash_enable_so" \
        -v enable_dylib="$bash_enable_dylib" '
        $0 != marker && $0 != enable_so && $0 != enable_dylib { print }
    ' "$BASHRC" > "$tmp"
    mv "$tmp" "$BASHRC"
    say "Removed flyline startup lines from ${BASHRC}"
}

# Install from locally-built artifacts (cargo build) instead of a release
# download. Symlinks the built binary/lib and the checkout's widget script into
# INSTALL_DIR, then wires up ~/.zshrc — so rebuilds are picked up automatically.
# Usage: sh install.sh --local [DIST_DIR]   (DIST_DIR defaults to target/release)
local_main() {
    # Resolve the checkout dir (where scripts/ and target/ live).
    if [ -n "${0:-}" ] && [ "$0" != "sh" ]; then
        REPO_DIR="$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)"
    else
        REPO_DIR="$(pwd)"
    fi

    if [ -n "${1:-}" ]; then
        DIST_DIR="$(expand_path "$1")"
    elif [ -n "${FLYLINE_LOCAL_DIST:-}" ]; then
        DIST_DIR="$(expand_path "$FLYLINE_LOCAL_DIST")"
    elif [ -x "${REPO_DIR}/target/release/${STANDALONE_BIN}" ]; then
        DIST_DIR="${REPO_DIR}/target/release"
    else
        DIST_DIR="${REPO_DIR}/target/debug"
    fi
    # Symlinks must be absolute: INSTALL_DIR is not the cwd, so a relative
    # DIST_DIR like `target/release` would resolve under INSTALL_DIR instead.
    DIST_DIR="$(CDPATH='' cd -- "$DIST_DIR" && pwd)"

    standalone_src="${DIST_DIR}/${STANDALONE_BIN}"
    if [ ! -x "$standalone_src" ]; then
        err "No ${STANDALONE_BIN} in ${DIST_DIR}. Build it first:
    cargo build --release --features standalone"
    fi

    if [ ! -f "${REPO_DIR}/scripts/flyline.zsh" ]; then
        err "Cannot find ${REPO_DIR}/scripts/flyline.zsh (run --local from the flyline checkout)."
    fi
    if [ ! -f "${REPO_DIR}/scripts/flyline.fish" ]; then
        err "Cannot find ${REPO_DIR}/scripts/flyline.fish (run --local from the flyline checkout)."
    fi

    mkdir -p "$INSTALL_DIR" "${INSTALL_DIR}/scripts"

    ln -sf "$standalone_src" "${INSTALL_DIR}/${STANDALONE_BIN}"
    say "Linked ${INSTALL_DIR}/${STANDALONE_BIN} -> ${standalone_src}"

    # Best-effort: link the Bash loadable too, if it was built.
    for lib in libflyline.so libflyline.dylib; do
        if [ -f "${DIST_DIR}/${lib}" ]; then
            ln -sf "${DIST_DIR}/${lib}" "${INSTALL_DIR}/${lib}"
            say "Linked ${INSTALL_DIR}/${lib} -> ${DIST_DIR}/${lib}"
        fi
    done

    ln -sf "${REPO_DIR}/scripts/flyline.zsh" "${INSTALL_DIR}/scripts/flyline.zsh"
    say "Linked ${INSTALL_DIR}/scripts/flyline.zsh -> ${REPO_DIR}/scripts/flyline.zsh"
    ln -sf "${REPO_DIR}/scripts/flyline.fish" "${INSTALL_DIR}/scripts/flyline.fish"
    say "Linked ${INSTALL_DIR}/scripts/flyline.fish -> ${REPO_DIR}/scripts/flyline.fish"

    if ! has_zsh && ! has_fish; then
        warn "Neither zsh nor fish found on PATH; installed files but skipped shell integration."
        return
    fi

    if has_zsh; then
        ensure_zshrc_block
    fi
    if has_fish; then
        write_fish_confd
    fi

    say ""
    say "Local install complete."
    if has_zsh; then
        say "    Activate now (zsh):  exec zsh"
    fi
    if has_fish; then
        say "    Activate now (fish): exec fish"
    fi
    say "    Run the tutorial:    flyline run-tutorial"
    say "    Disable in session:  flyline_disable"
    say "    Uninstall:           sh install.sh --uninstall"
    say "    Symlinks mean rebuilds are picked up automatically (no re-install)."
}

uninstall_main() {
    say "Uninstalling flyline..."
    remove_zshrc_flyline_block
    remove_bashrc_flyline_lines
    remove_fish_integration

    # These generically named license files are part of the release archive.
    # Only remove them when the adjacent flyline provenance file confirms this
    # directory contains a packaged flyline installation.
    remove_release_metadata=false
    if [ -f "${INSTALL_DIR}/UPSTREAM_BASE.toml" ]; then
        remove_release_metadata=true
    fi

    removed_files=false
    for path in \
        "${INSTALL_DIR}/${STANDALONE_BIN}" \
        "${INSTALL_DIR}/scripts/flyline.zsh" \
        "${INSTALL_DIR}/scripts/flyline.fish" \
        "${INSTALL_DIR}/libflyline.so" \
        "${INSTALL_DIR}"/libflyline.so.* \
        "${INSTALL_DIR}/libflyline.dylib" \
        "${INSTALL_DIR}"/libflyline.dylib.* \
        "${INSTALL_DIR}/UPSTREAM_BASE.toml"; do
        if [ -e "$path" ] || [ -L "$path" ]; then
            rm -f "$path"
            removed_files=true
        fi
    done
    if $remove_release_metadata; then
        rm -f "${INSTALL_DIR}/LICENSE-MIT" "${INSTALL_DIR}/LICENSE-GPLv3"
    fi
    rmdir "${INSTALL_DIR}/scripts" 2>/dev/null || true
    if $removed_files; then
        say "Removed flyline executables, libraries, integration scripts, and release metadata from ${INSTALL_DIR}"
    else
        say "No installed flyline files were found in ${INSTALL_DIR}"
    fi

    say ""
    say "Uninstall complete. Existing shells keep already-loaded commands until they are restarted."
    say "    zsh: open a new terminal, or run:"
    say "         unfunction flyline flyline_enable flyline_disable flyline_uninstall _flyline_edit 2>/dev/null"
    say "    fish: open a new terminal, or run: flyline_uninstall"
    say "    Bash: open a new terminal, or run:"
    say "          enable -d flyline"
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

main() {
    OS="$(detect_os)"
    ARCH="$(detect_arch)"
    install_bash_integration=true

    if is_system_bash_pre_4_4; then
        use_bash_pre_4_4=true
    else
        use_bash_pre_4_4=false
    fi

    if [ "$OS" = "darwin" ]; then
        TARGET="${ARCH}-apple-darwin"
        LIB_NAME="libflyline.dylib"

        # Flyline can run on the 3.2.57 version of Bash.
        # However, the Bash binary on macOS is often compiled without linkable symbols required to load the Flyline plugin.
        if $use_bash_pre_4_4; then
            BREW_BASH="$(find_homebrew_bash)"
            if [ -n "$BREW_BASH" ]; then
                warn "Your system Bash is older than 4.4. This version won't have been compiled with custom plugin support."
                warn "Ensure that you use $BREW_BASH for flyline."
                use_bash_pre_4_4=false
            elif has_zsh; then
                warn "Stock macOS Bash cannot load the flyline builtin and no supported Homebrew Bash was found."
                warn "Continuing with zsh-only installation; Bash integration will be skipped."
                use_bash_pre_4_4=false
                install_bash_integration=false
            else
                err_no_exit "Your system Bash is older than 4.4. This version won't have been compiled with custom plugin support."
                err_no_exit "Please install a newer Bash before trying to use flyline:"
                err "    brew install bash"
            fi
        fi
    elif [ "$OS" = "freebsd" ]; then
        if [ "$ARCH" != "x86_64" ]; then
            err "Unsupported FreeBSD architecture: $ARCH. Only x86_64 is supported."
        fi
        TARGET="x86_64-unknown-freebsd"
        LIB_NAME="libflyline.so"
    elif [ "$OS" = "android" ]; then
        case "$ARCH" in
            aarch64)
                TARGET="aarch64-linux-android"
                ;;
            x86_64)
                TARGET="x86_64-linux-android"
                ;;
            armv7)
                TARGET="armv7-linux-androideabi"
                ;;
            i686)
                TARGET="i686-linux-android"
                ;;
            *)
                err "Unsupported Android architecture: $ARCH"
                ;;
        esac
        LIB_NAME="libflyline.so"
    else
        LIBC="$(detect_libc)"
        case "$ARCH" in
            armv7)
                if [ "$LIBC" = "gnu" ]; then
                    TARGET="armv7-unknown-linux-gnueabihf"
                else
                    err "Unsupported libc ($LIBC) for armv7. Only gnu (gnueabihf) is supported."
                fi
                ;;
            *)
                TARGET="${ARCH}-unknown-linux-${LIBC}"
                ;;
        esac
        LIB_NAME="libflyline.so"
    fi

    say "Detected target: ${TARGET}"

    # Reject targets/combinations for which no release archive is built, rather
    # than constructing a URL (or asset path) that cannot resolve.
    if $use_bash_pre_4_4; then
        if ! is_supported_pre_bash_4_4_target "$TARGET"; then
            if has_zsh; then
                warn "No pre-Bash-4.4 builtin is available for ${TARGET}."
                warn "Continuing with zsh-only installation; install Bash >= 4.4 to enable Bash integration."
                use_bash_pre_4_4=false
                install_bash_integration=false
            else
                err "No pre-Bash-4.4 build is available for ${TARGET}. The _pre_bash_4_4 archive is only built for x86_64-unknown-linux-gnu. Install Bash >= 4.4 to use flyline on this platform."
            fi
        fi
    fi
    if ! $use_bash_pre_4_4 && ! is_supported_target "$TARGET"; then
        err "No release archive is built for target ${TARGET} (unsupported architecture/libc combination)."
    fi

    if [ -n "${FLYLINE_INSTALL_VERSION:-}" ]; then
        say "Using specified release version: ${FLYLINE_INSTALL_VERSION}"
        VERSION="${FLYLINE_INSTALL_VERSION}"
    elif [ -n "$FLYLINE_ASSET_BASE" ]; then
        err "FLYLINE_ASSET_BASE is set but no version was specified. Set FLYLINE_INSTALL_VERSION to the version of the assets in ${FLYLINE_ASSET_BASE}."
    else
        channel="${FLYLINE_CHANNEL:-stable}"
        say "Fetching ${channel} release information..."
        VERSION="$(get_channel_version "$channel")"
        say "Using ${channel} version: ${VERSION}"
    fi

    ARCHIVE_STEM="libflyline-${VERSION}-${TARGET}"

    if $use_bash_pre_4_4; then
        say "Detected Bash < 4.4, using pre-bash-4.4 build..."
        ARCHIVE="${ARCHIVE_STEM}_pre_bash_4_4.tar.gz"
        ARCHIVE_SHA256="${ARCHIVE}.sha256"
    else
        ARCHIVE="${ARCHIVE_STEM}.tar.gz"
        ARCHIVE_SHA256="${ARCHIVE}.sha256"
    fi

    TMP_DIR="$(mktemp -d)"
    trap cleanup_install_tmp EXIT

    if [ -n "$FLYLINE_ASSET_BASE" ]; then
        say "Fetching ${ARCHIVE} from asset base ${FLYLINE_ASSET_BASE}..."
    else
        say "Downloading ${ARCHIVE} from
    https://github.com/${REPO}/releases/download/${VERSION}/${ARCHIVE}..."
    fi
    fetch_asset "$ARCHIVE" "${TMP_DIR}/${ARCHIVE}"

    say "Fetching checksum ${ARCHIVE_SHA256}..."
    fetch_asset "$ARCHIVE_SHA256" "${TMP_DIR}/${ARCHIVE_SHA256}"

    say "Verifying checksum..."
    # Run from TMP_DIR so the relative path in the checksum file resolves.
    (cd "$TMP_DIR" && verify_sha256 "$ARCHIVE_SHA256") \
        || err "Checksum verification failed for ${ARCHIVE}."


    mkdir -p "$INSTALL_DIR"

    # Unpack into a staging directory and copy from there, rather than unpacking
    # straight over the files a previous install is still using.
    STAGE="${TMP_DIR}/pkg"
    mkdir -p "$STAGE"
    tar xzf "${TMP_DIR}/${ARCHIVE}" -C "$STAGE"

    # Resolving this must be fatal: an empty suffix would look for
    # "${LIB_NAME}." and fall through to leaving a previous install's symlink
    # (and therefore an older library) in place.
    VERSION_NO_V="$(lib_version_suffix "$VERSION" "$STAGE" "$LIB_NAME")" \
        || err "Could not determine the packaged ${LIB_NAME} version from ${ARCHIVE}. The archive must contain exactly one ${LIB_NAME}.<version> file."
    LIB_VERSIONED="${LIB_NAME}.${VERSION_NO_V}"

    install_staged_tree "$STAGE" "$INSTALL_DIR"

    if [ -f "${INSTALL_DIR}/${LIB_VERSIONED}" ]; then
        say "Creating symlink ${LIB_NAME} -> ${LIB_VERSIONED}..."
        rm -f "${INSTALL_DIR}/${LIB_NAME}"
        (cd "$INSTALL_DIR" && ln -s "$LIB_VERSIONED" "$LIB_NAME")
    elif [ -f "${STAGE}/${LIB_NAME}" ]; then
        # The archive itself shipped an unversioned library, which the copy above
        # already put in place. Checking the staging directory rather than
        # INSTALL_DIR keeps this distinct from a previous install's leftover
        # symlink, which would otherwise satisfy the same test and leave the
        # older library in use.
        warn "Archive contains ${LIB_NAME} rather than ${LIB_VERSIONED}; using it as installed."
    else
        err "Failed to install ${LIB_VERSIONED} into ${INSTALL_DIR}."
    fi

    LIB_PATH="${INSTALL_DIR}/${LIB_NAME}"
    say "Installed: ${LIB_PATH}"

    if [ -f "${INSTALL_DIR}/${STANDALONE_BIN}" ]; then
        chmod +x "${INSTALL_DIR}/${STANDALONE_BIN}"
    fi

    install_zsh_integration
    install_fish_integration

    # A bad `enable -f` line in ~/.bashrc runs on every new interactive Bash, so
    # look for evidence the builtin cannot load before writing it. The probe is a
    # separate Bash process with --norc, so it reads no user config and cannot
    # disturb the running shell. It needs -i because flyline deliberately
    # declines to load in non-interactive shells, which makes the probe's exit
    # status on its own a false negative for a perfectly good library.
    #
    # Only act on positive evidence of an unloadable library: a dlopen or symbol
    # error, or the probe dying from a signal as it does on a truncated object.
    # Anything else counts as a probe that could not draw a conclusion, and the
    # integration is written as before.
    if $install_bash_integration && command -v bash >/dev/null 2>&1; then
        probe_status=0
        # LC_ALL=C because the decision below reads Bash's own error text, which
        # is translated under a localized locale.
        probe_out="$(LC_ALL=C bash --norc -i -c 'enable -f "$1" flyline' \
            flyline-load-probe "$LIB_PATH" 2>&1)" || probe_status=$?
        if [ "$probe_status" -ne 0 ] && {
            [ "$probe_status" -ge 128 ] \
                || printf '%s' "$probe_out" \
                    | grep -q 'cannot open shared object\|cannot find flyline_struct'
        }; then
            install_bash_integration=false
            probe_detail="$(printf '%s' "$probe_out" | grep 'enable:' | head -1)"
            warn "Bash could not load ${LIB_PATH}${probe_detail:+ (${probe_detail})}."
            warn "Leaving ${BASHRC} unchanged so your terminals keep starting normally."
            warn "Once the cause is resolved, enable it with:"
            warn "    enable -f ${LIB_PATH} flyline"
        fi
    fi

    # Update or add 'enable -f ... flyline' in ~/.bashrc when this platform's
    # Bash can load the packaged builtin.
    if $install_bash_integration; then
        ENABLE_CMD="enable -f ${LIB_PATH} flyline"
        if [ -z "${FLYLINE_VERSION:-}" ]; then
            printf '\n%s\n%s\n' "$FLYLINE_BASHRC_MARKER" "$ENABLE_CMD" >> "$BASHRC"
            say "Added flyline to ${BASHRC}"
        else
            say "Flyline is already installed (detected ${FLYLINE_VERSION}); skipping .bashrc modification."
        fi
    else
        say "Skipped Bash integration on this platform."
    fi


    # On macOS, login shells read ~/.bash_profile (not ~/.bashrc).
    # Warn the user if ~/.bash_profile does not appear to source ~/.bashrc.
    if [ "$OS" = "darwin" ] && $install_bash_integration; then
        BASH_PROFILE="${HOME}/.bash_profile"
        if [ -f "$BASH_PROFILE" ]; then
            if ! grep -qE '(source|\.)[[:space:]]+(~|\$\{?HOME\}?)/\.bashrc([[:space:]]|$)' "$BASH_PROFILE"; then
                warn "Your ${BASH_PROFILE} does not appear to source ~/.bashrc."
                warn "On macOS, login shells read ~/.bash_profile, so flyline may not load in new terminals."
                warn "Consider adding the following to ${BASH_PROFILE}:"
                warn '    if [ -f ~/.bashrc ]; then . ~/.bashrc; fi'
            fi
        else
            warn "${BASH_PROFILE} does not exist."
            warn "On macOS, login shells read ~/.bash_profile, so flyline may not load in new terminals."
            warn "Consider creating ${BASH_PROFILE} with the following content:"
            warn '    if [ -f ~/.bashrc ]; then . ~/.bashrc; fi'
        fi
    fi

    say ""
    if [ -n "${FLYLINE_VERSION:-}" ]; then
        say "Upgrade from ${FLYLINE_VERSION} -> ${VERSION}, run \`flyline changelog\` to see what's changed."
        say "To activate the upgrade, open a new shell."
        if [ -n "${FLYLINE_LOAD_DIR:-}" ]; then
            resolved_load_dir="$(expand_path "$FLYLINE_LOAD_DIR")"
            if [ "$resolved_load_dir" != "$INSTALL_DIR" ]; then
                warn "The upgrade installation directory ($INSTALL_DIR) is different from the currently running load directory ($resolved_load_dir)."
                warn "Please make sure to update your ~/.bashrc or other startup scripts to point to the new libflyline."
            fi
        fi
    else
        say "Installation complete!"
        if $install_bash_integration; then
            say '    To activate in the current Bash shell:'
            if [ -z "${FLYLINE_INSTALL_DIR:-}" ]; then
                say "        $ENABLE_CMD"
            else
                say "        enable -d flyline && enable -f ${LIB_PATH} flyline"
            fi
        fi
        if has_zsh && [ -f "${INSTALL_DIR}/${STANDALONE_BIN}" ]; then
            say '    For zsh, open a new terminal (or run: exec zsh).'
        fi
        if has_fish && [ -f "${INSTALL_DIR}/${STANDALONE_BIN}" ]; then
            say '    For fish, open a new terminal (or run: exec fish).'
        fi
        say '    Or open a new terminal and run the tutorial:'
        say "        flyline run-tutorial"
    fi

    # Detect if ble.sh is running or configured in ~/.bashrc
    if [ -n "${_ble_version:-}" ] || { [ -f "$BASHRC" ] && grep -q 'ble\.sh' "$BASHRC"; }; then
        say ""
        warn "ble.sh (Bash Line Editor) is detected."
        warn "Please turn it off/disable it before starting flyline to avoid conflicts."
    fi
}

# Tests source this script with FLYLINE_INSTALL_SH_LIB=1 to reach the helper
# functions without installing anything. Executing it with the variable set
# would otherwise be a silent no-op, so reject that instead.
flyline_lib_mode=false
if [ "${FLYLINE_INSTALL_SH_LIB:-}" = 1 ]; then
    case "${0##*/}" in
        install.sh | sh | bash | dash | ash | busybox)
            err "FLYLINE_INSTALL_SH_LIB=1 only applies when install.sh is sourced by the test suite. Unset it to install."
            ;;
        *)
            flyline_lib_mode=true
            ;;
    esac
fi

if ! $flyline_lib_mode; then
    case "${1:-}" in
        --uninstall|-u)
            if [ -n "${FLYLINE_INSTALL_DIR:-}" ]; then
                INSTALL_DIR="$(expand_path "$FLYLINE_INSTALL_DIR")"
            elif [ -n "${FLYLINE_LOAD_DIR:-}" ]; then
                INSTALL_DIR="$(expand_path "$FLYLINE_LOAD_DIR")"
            fi
            uninstall_main
            ;;
        --local|-l)
            local_main "${2:-}"
            ;;
        *)
            main "$@"
            ;;
    esac
fi
