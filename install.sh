#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:-}"
REPO="razorback16/codiv"
INSTALL_DIR="$HOME/.local/bin"
PLIST_LABEL="ai.codiv.daemon"
PLIST_PATH="$HOME/Library/LaunchAgents/${PLIST_LABEL}.plist"
SYSTEMD_DIR="$HOME/.config/systemd/user"
SYSTEMD_UNIT="codivd.service"

# --- Helpers ---

info()  { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
warn()  { printf '\033[1;33mWarning:\033[0m %s\n' "$*" >&2; }
error() { printf '\033[1;31mError:\033[0m %s\n' "$*" >&2; exit 1; }

download() {
    local url="$1" output="${2:-}"
    if command -v curl >/dev/null 2>&1; then
        if [ -n "$output" ]; then
            curl -fsSL -o "$output" "$url"
        else
            curl -fsSL "$url"
        fi
    elif command -v wget >/dev/null 2>&1; then
        if [ -n "$output" ]; then
            wget -q -O "$output" "$url"
        else
            wget -q -O - "$url"
        fi
    else
        error "Either curl or wget is required"
    fi
}

sha256_check() {
    local file="$1"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$file" | cut -d' ' -f1
    else
        shasum -a 256 "$file" | cut -d' ' -f1
    fi
}

# --- Platform Detection ---

detect_platform() {
    case "$(uname -s)" in
        Darwin) OS="darwin" ;;
        Linux)  OS="linux" ;;
        *)      error "Unsupported OS: $(uname -s)" ;;
    esac

    case "$(uname -m)" in
        x86_64|amd64)   ARCH="x86_64" ;;
        arm64|aarch64)  ARCH="aarch64" ;;
        *)              error "Unsupported architecture: $(uname -m)" ;;
    esac

    # Rosetta 2 detection
    if [ "$OS" = "darwin" ] && [ "$ARCH" = "x86_64" ]; then
        if [ "$(sysctl -n sysctl.proc_translated 2>/dev/null)" = "1" ]; then
            ARCH="aarch64"
        fi
    fi

    case "$OS" in
        darwin) TARGET="${ARCH}-apple-darwin" ;;
        linux)  TARGET="${ARCH}-unknown-linux-gnu" ;;
    esac
}

# --- Version Resolution ---

resolve_version() {
    if [ -n "$VERSION" ]; then
        info "Installing codiv $VERSION"
        return
    fi
    info "Fetching latest version..."
    local response
    response=$(download "https://api.github.com/repos/${REPO}/releases/latest") \
        || error "Could not fetch release info (GitHub API may be rate-limited)"
    VERSION=$(echo "$response" | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/' || true)
    [ -n "$VERSION" ] || error "Could not determine latest version"
    info "Latest version: $VERSION"
}

# --- Download & Verify ---

download_and_verify() {
    local tarball_name="codiv-${VERSION}-${TARGET}.tar.gz"
    local url_base="https://github.com/${REPO}/releases/download/${VERSION}"

    TMPDIR_INSTALL="$(mktemp -d)"
    trap 'rm -rf "$TMPDIR_INSTALL"' EXIT

    info "Downloading checksums..."
    download "${url_base}/checksums.txt" "$TMPDIR_INSTALL/checksums.txt"

    info "Downloading ${tarball_name}..."
    download "${url_base}/${tarball_name}" "$TMPDIR_INSTALL/${tarball_name}"

    info "Verifying checksum..."
    local expected
    expected=$(grep "$tarball_name" "$TMPDIR_INSTALL/checksums.txt" | awk '{print $1}')
    [ -n "$expected" ] || error "Checksum not found for $tarball_name"

    local actual
    actual=$(sha256_check "$TMPDIR_INSTALL/${tarball_name}")
    [ "$actual" = "$expected" ] || error "Checksum mismatch: expected $expected, got $actual"

    info "Extracting binaries..."
    tar xzf "$TMPDIR_INSTALL/${tarball_name}" -C "$TMPDIR_INSTALL"

    mkdir -p "$INSTALL_DIR"
    local extract_dir="$TMPDIR_INSTALL/codiv-${VERSION}-${TARGET}"
    cp "$extract_dir/codiv" "$INSTALL_DIR/codiv"
    cp "$extract_dir/codivd" "$INSTALL_DIR/codivd"
    chmod +x "$INSTALL_DIR/codiv" "$INSTALL_DIR/codivd"
}

# --- PATH Setup ---

setup_path() {
    if echo "$PATH" | tr ':' '\n' | grep -qx "$HOME/.local/bin"; then
        return
    fi

    info "Adding ~/.local/bin to PATH..."
    local shell_name
    shell_name="$(basename "$SHELL")"
    local line='export PATH="$HOME/.local/bin:$PATH"'

    case "$shell_name" in
        zsh)
            echo "" >> "$HOME/.zshrc"
            echo "# Added by codiv installer" >> "$HOME/.zshrc"
            echo "$line" >> "$HOME/.zshrc"
            ;;
        bash)
            echo "" >> "$HOME/.bashrc"
            echo "# Added by codiv installer" >> "$HOME/.bashrc"
            echo "$line" >> "$HOME/.bashrc"
            ;;
        fish)
            mkdir -p "$HOME/.config/fish"
            echo "" >> "$HOME/.config/fish/config.fish"
            echo "# Added by codiv installer" >> "$HOME/.config/fish/config.fish"
            echo "fish_add_path $HOME/.local/bin" >> "$HOME/.config/fish/config.fish"
            ;;
        *)
            warn "Unknown shell '$shell_name'. Please add ~/.local/bin to your PATH manually."
            ;;
    esac
}

# --- Service Setup ---

stop_existing_service() {
    if [ "$OS" = "darwin" ]; then
        launchctl bootout "gui/$(id -u)/${PLIST_LABEL}" 2>/dev/null || true
    else
        systemctl --user stop "$SYSTEMD_UNIT" 2>/dev/null || true
    fi
}

setup_service_macos() {
    info "Setting up launchd service..."
    mkdir -p "$(dirname "$PLIST_PATH")"
    mkdir -p "$HOME/.codiv"

    cat > "$PLIST_PATH" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>${PLIST_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>${INSTALL_DIR}/codivd</string>
        <string>--foreground</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
</dict>
</plist>
EOF

    launchctl bootstrap "gui/$(id -u)" "$PLIST_PATH"
}

setup_service_linux() {
    info "Setting up systemd user service..."
    mkdir -p "$SYSTEMD_DIR"
    mkdir -p "$HOME/.codiv"

    cat > "${SYSTEMD_DIR}/${SYSTEMD_UNIT}" << EOF
[Unit]
Description=Codiv AI Daemon
After=default.target

[Service]
ExecStart=%h/.local/bin/codivd --foreground
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
EOF

    systemctl --user daemon-reload
    systemctl --user enable --now "$SYSTEMD_UNIT"
    loginctl enable-linger "$(whoami)" 2>/dev/null || true
}

setup_service() {
    stop_existing_service
    if [ "$OS" = "darwin" ]; then
        setup_service_macos
    else
        setup_service_linux
    fi
}

# --- Main ---

main() {
    detect_platform
    resolve_version
    download_and_verify
    setup_path
    setup_service

    echo ""
    info "Codiv $VERSION installed successfully!"
    echo ""
    echo "  Binary:  $INSTALL_DIR/codiv"
    echo "  Daemon:  $INSTALL_DIR/codivd (running as service)"
    echo "  Config:  ~/.codiv/config.toml"
    echo "  Logs:    ~/.codiv/codivd.log"
    echo ""

    if ! echo "$PATH" | tr ':' '\n' | grep -qx "$HOME/.local/bin"; then
        echo "  Restart your shell or run:"
        echo "    export PATH=\"\$HOME/.local/bin:\$PATH\""
        echo ""
    fi

    echo "  Get started:"
    echo "    codiv"
    echo ""
    echo "  Uninstall:"
    if [ "$OS" = "darwin" ]; then
        echo "    launchctl bootout gui/\$(id -u)/${PLIST_LABEL}"
        echo "    rm ${PLIST_PATH}"
    else
        echo "    systemctl --user disable --now ${SYSTEMD_UNIT}"
        echo "    rm ${SYSTEMD_DIR}/${SYSTEMD_UNIT}"
        echo "    systemctl --user daemon-reload"
    fi
    echo "    rm ~/.local/bin/codiv ~/.local/bin/codivd"
}

main
