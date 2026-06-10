#!/bin/sh
set -eu

REPO="VinayIN/cite-cli"
BIN_NAME="cite-cli"
VERSION="0.1.0"

GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

info()  { printf "${BLUE}%s${NC}\n" "$*"; }
ok()    { printf "${GREEN}%s${NC}\n" "$*"; }
warn()  { printf "${YELLOW}%s${NC}\n" "$*"; }
err()   { printf "${RED}%s${NC}\n" "$*"; }

detect_arch() {
    arch=$(uname -m)
    case "$arch" in
        x86_64|amd64) echo "x86_64" ;;
        aarch64|arm64) echo "aarch64" ;;
        *) echo "$arch" ;;
    esac
}

detect_os() {
    os=$(uname -s)
    case "$os" in
        Linux)   echo "unknown-linux-gnu" ;;
        Darwin)  echo "apple-darwin" ;;
        MINGW*|MSYS*|CYGWIN*) echo "pc-windows-msvc" ;;
        *)       echo "$os" ;;
    esac
}

install_from_source() {
    info "Building $BIN_NAME v$VERSION from source..."
    if ! command -v cargo >/dev/null 2>&1; then
        err "Rust is required to build from source."
        err "Install it via: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
        exit 1
    fi

    tmpdir=$(mktemp -d 2>/dev/null || mktemp -d -t cite-install)
    info "Cloning $REPO into $tmpdir ..."
    git clone --depth 1 --branch "v$VERSION" "https://github.com/$REPO.git" "$tmpdir" 2>/dev/null || \
        git clone --depth 1 "https://github.com/$REPO.git" "$tmpdir"

    info "Running cargo build --release ..."
    (cd "$tmpdir" && cargo build --release)

    cp "$tmpdir/target/release/$BIN_NAME" "$1"
    rm -rf "$tmpdir"
    ok "Built $BIN_NAME v$VERSION from source."
}

install_from_release() {
    url="$1"
    dest="$2"
    info "Downloading $BIN_NAME v$VERSION for $(uname -s)/$(uname -m) ..."
    if command -v curl >/dev/null 2>&1; then
        curl -sSfL "$url" -o "$dest"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "$dest" "$url"
    else
        return 1
    fi
    chmod +x "$dest"
    ok "Downloaded $BIN_NAME v$VERSION."
    return 0
}

main() {
    arch=$(detect_arch)
    os=$(detect_os)
    target="${arch}-${os}"

    info "Detected: $(uname -s) / $(uname -m)  →  $target"

    if command -v "$BIN_NAME" >/dev/null 2>&1; then
        existing_path=$(command -v "$BIN_NAME")
        info "$BIN_NAME is already installed at $existing_path"
    fi

    if [ -n "${CARGO_HOME-}" ] && [ -f "${CARGO_HOME}/bin/$BIN_NAME" ]; then
        existing_path="${CARGO_HOME}/bin/$BIN_NAME"
        info "$BIN_NAME is already installed at $existing_path"
    fi

    if [ "$(id -u)" = 0 ]; then
        dest="/usr/local/bin/$BIN_NAME"
    else
        dest="${HOME}/.local/bin/$BIN_NAME"
    fi
    dest_dir=$(dirname "$dest")
    mkdir -p "$dest_dir"

    ext=""
    case "$os" in
        *windows*) ext=".exe" ;;
    esac
    dest="${dest}${ext}"

    release_url="https://github.com/$REPO/releases/download/v${VERSION}/${BIN_NAME}-${target}${ext}"
    download_ok=false

    if command -v curl >/dev/null 2>&1 || command -v wget >/dev/null 2>&1; then
        if install_from_release "$release_url" "$dest"; then
            download_ok=true
        else
            warn "Pre-built binary not available for $target (or GitHub unreachable)."
        fi
    else
        warn "Neither curl nor wget found; will build from source."
    fi

    if [ "$download_ok" = false ]; then
        install_from_source "$dest"
    fi

    if ! echo "$PATH" | tr ':' '\n' | grep -qx "$dest_dir"; then
        warn "Add $dest_dir to your PATH (e.g. export PATH=\"\$PATH:$dest_dir\")"
    fi

    ok "$BIN_NAME v$VERSION installed to $dest"
    info "Run '$BIN_NAME --help' to get started."
}

main "$@"
