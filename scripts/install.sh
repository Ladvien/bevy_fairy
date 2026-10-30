#!/bin/bash
# Universal installer for bevy-fairy
# Supports macOS, Linux, and WSL

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
REPO_URL="https://github.com/ladvien/bevy_fairy"
INSTALL_DIR="/usr/local/bin"
CONFIG_DIR="$HOME/.config/bevy-fairy"
BINARY_NAME="bevy-fairy"

# Print colored output
print_info() { echo -e "${BLUE}[INFO]${NC} $1"; }
print_success() { echo -e "${GREEN}[✓]${NC} $1"; }
print_warn() { echo -e "${YELLOW}[!]${NC} $1"; }
print_error() { echo -e "${RED}[✗]${NC} $1"; }

# Detect OS and architecture
detect_platform() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"
    
    case "$OS" in
        Darwin)
            PLATFORM="apple-darwin"
            case "$ARCH" in
                arm64|aarch64) ARCH="aarch64" ;;
                x86_64) ARCH="x86_64" ;;
                *) print_error "Unsupported architecture: $ARCH"; exit 1 ;;
            esac
            ;;
        Linux)
            PLATFORM="unknown-linux-gnu"
            case "$ARCH" in
                x86_64) ARCH="x86_64" ;;
                aarch64) ARCH="aarch64" ;;
                *) print_error "Unsupported architecture: $ARCH"; exit 1 ;;
            esac
            ;;
        MINGW*|MSYS*|CYGWIN*)
            print_warn "Windows detected. Please use install.ps1 instead."
            exit 1
            ;;
        *)
            print_error "Unsupported OS: $OS"
            exit 1
            ;;
    esac
    
    PLATFORM_STRING="${ARCH}-${PLATFORM}"
    print_info "Detected platform: $PLATFORM_STRING"
}

# Check for required tools
check_requirements() {
    print_info "Checking requirements..."
    
    # Check for curl or wget
    if command -v curl &> /dev/null; then
        DOWNLOAD_CMD="curl -sSL"
        DOWNLOAD_OUTPUT="-o"
    elif command -v wget &> /dev/null; then
        DOWNLOAD_CMD="wget -q"
        DOWNLOAD_OUTPUT="-O"
    else
        print_error "Neither curl nor wget found. Please install one of them."
        exit 1
    fi
    
    # Check for tar
    if ! command -v tar &> /dev/null; then
        print_error "tar is required but not installed."
        exit 1
    fi
    
    print_success "All requirements met"
}

# Download and install binary
install_binary() {
    print_info "Downloading $BINARY_NAME..."
    
    # Create temp directory
    TEMP_DIR=$(mktemp -d)
    trap "rm -rf $TEMP_DIR" EXIT
    
    # Determine download URL
    RELEASE_URL="${REPO_URL}/releases/latest/download/${BINARY_NAME}-${PLATFORM_STRING}.tar.gz"
    
    # Download binary
    if ! $DOWNLOAD_CMD "$RELEASE_URL" $DOWNLOAD_OUTPUT "$TEMP_DIR/bevy-fairy.tar.gz"; then
        print_warn "Failed to download pre-built binary. Trying to build from source..."
        build_from_source
        return
    fi
    
    # Extract binary
    tar -xzf "$TEMP_DIR/bevy-fairy.tar.gz" -C "$TEMP_DIR"
    
    # Install binary
    if [ -w "$INSTALL_DIR" ]; then
        mv "$TEMP_DIR/$BINARY_NAME" "$INSTALL_DIR/"
    else
        print_info "Installing to $INSTALL_DIR requires sudo permission"
        sudo mv "$TEMP_DIR/$BINARY_NAME" "$INSTALL_DIR/"
    fi
    
    # Make executable
    chmod +x "$INSTALL_DIR/$BINARY_NAME"
    
    print_success "Binary installed to $INSTALL_DIR/$BINARY_NAME"
}

# Build from source as fallback
build_from_source() {
    print_info "Building from source..."
    
    # Check for Rust
    if ! command -v cargo &> /dev/null; then
        print_warn "Rust not found. Installing Rust..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        source "$HOME/.cargo/env"
    fi
    
    # Clone and build
    TEMP_DIR=$(mktemp -d)
    trap "rm -rf $TEMP_DIR" EXIT
    
    git clone "$REPO_URL" "$TEMP_DIR/bevy-fairy" || {
        print_error "Failed to clone repository"
        exit 1
    }
    
    cd "$TEMP_DIR/bevy-fairy"
    # Only the server half. The plugin half needs Bevy, and an agent installing the server
    # does not want to compile the engine.
    cargo build --release --no-default-features --features server

    # Install binary
    if [ -w "$INSTALL_DIR" ]; then
        mv "target/release/bevy-fairy" "$INSTALL_DIR/$BINARY_NAME"
    else
        print_info "Installing to $INSTALL_DIR requires sudo permission"
        sudo mv "target/release/bevy-fairy" "$INSTALL_DIR/$BINARY_NAME"
    fi
    
    chmod +x "$INSTALL_DIR/$BINARY_NAME"
    print_success "Built and installed from source"
}

# Setup configuration
setup_config() {
    print_info "Setting up configuration..."
    
    # Create config directory
    mkdir -p "$CONFIG_DIR"
    
    # Create default config if it doesn't exist
    if [ ! -f "$CONFIG_DIR/config.toml" ]; then
        cat > "$CONFIG_DIR/config.toml" << 'EOF'
[connection]
bevy_host = "localhost"
bevy_port = 15702
mcp_port = 3000

[logging]
level = "info"
debug_mode = false

[features]
auto_reconnect = true
checkpoint_interval = 1000
max_recording_size = "100MB"
EOF
        print_success "Created config file at $CONFIG_DIR/config.toml"
    else
        print_info "Config file already exists"
    fi
}

# Setup Claude Desktop integration
setup_claude() {
    print_info "Setting up Claude Desktop integration..."
    
    # Detect Claude config location
    if [ "$OS" = "Darwin" ]; then
        CLAUDE_CONFIG="$HOME/Library/Application Support/Claude/claude_desktop_config.json"
    else
        CLAUDE_CONFIG="$HOME/.config/Claude/claude_desktop_config.json"
    fi
    
    # Check if Claude is installed
    if [ ! -d "$(dirname "$CLAUDE_CONFIG")" ]; then
        print_warn "Claude Desktop not found. Please install it first."
        print_info "You can configure it later with scripts/setup-claude.sh, or add $BINARY_NAME to your MCP client by hand"
        return
    fi
    
    # Create or update config
    if [ -f "$CLAUDE_CONFIG" ]; then
        print_warn "Claude config exists. Backing up to $CLAUDE_CONFIG.bak"
        cp "$CLAUDE_CONFIG" "$CLAUDE_CONFIG.bak"
    fi
    
    # Create new config with MCP server
    cat > "$CLAUDE_CONFIG" << EOF
{
  "mcpServers": {
    "bevy-fairy": {
      "command": "$INSTALL_DIR/$BINARY_NAME",
      "args": ["--stdio"],
      "env": {
        "RUST_LOG": "info"
      }
    }
  }
}
EOF
    
    print_success "Claude Desktop configured"
    print_info "Please restart Claude Desktop to apply changes"
}

# Add to PATH if needed
setup_path() {
    print_info "Checking PATH..."
    
    if ! echo "$PATH" | grep -q "$INSTALL_DIR"; then
        print_warn "$INSTALL_DIR is not in PATH"
        
        # Detect shell
        SHELL_NAME="$(basename "$SHELL")"
        case "$SHELL_NAME" in
            bash)
                RC_FILE="$HOME/.bashrc"
                ;;
            zsh)
                RC_FILE="$HOME/.zshrc"
                ;;
            fish)
                RC_FILE="$HOME/.config/fish/config.fish"
                ;;
            *)
                RC_FILE="$HOME/.profile"
                ;;
        esac
        
        print_info "Adding to $RC_FILE"
        echo "export PATH=\"$INSTALL_DIR:\$PATH\"" >> "$RC_FILE"
        print_success "Added to PATH. Run 'source $RC_FILE' or restart your terminal"
    else
        print_success "PATH already configured"
    fi
}

# Verify installation
verify_installation() {
    print_info "Verifying installation..."
    
    if command -v "$BINARY_NAME" &> /dev/null; then
        VERSION=$("$BINARY_NAME" --version 2>/dev/null || echo "unknown")
        print_success "Installation successful! Version: $VERSION"
        
        echo ""
        print_success "🎉 bevy-fairy installed successfully!"
        echo ""
        echo "Next steps:"
        echo "  1. Add RemotePlugin to your Bevy game"
        echo "  2. Restart Claude Desktop"
        echo "  3. Start debugging with: $BINARY_NAME serve"
        echo ""
        echo "For more information, run: $BINARY_NAME --help"
    else
        print_error "Installation verification failed"
        echo "Please check that $INSTALL_DIR is in your PATH"
        exit 1
    fi
}

# Uninstall function
uninstall() {
    print_info "Uninstalling bevy-fairy..."
    
    # Remove binary
    if [ -f "$INSTALL_DIR/$BINARY_NAME" ]; then
        if [ -w "$INSTALL_DIR" ]; then
            rm "$INSTALL_DIR/$BINARY_NAME"
        else
            sudo rm "$INSTALL_DIR/$BINARY_NAME"
        fi
        print_success "Removed binary"
    fi
    
    # Remove config
    if [ -d "$CONFIG_DIR" ]; then
        print_warn "Remove config directory? (y/N)"
        read -r response
        if [[ "$response" =~ ^[Yy]$ ]]; then
            rm -rf "$CONFIG_DIR"
            print_success "Removed config directory"
        fi
    fi
    
    print_success "Uninstall complete"
}

# Main installation flow
main() {
    echo ""
    echo "╔══════════════════════════════════════════╗"
    echo "║   bevy-fairy Installer    ║"
    echo "╚══════════════════════════════════════════╝"
    echo ""
    
    # Check for uninstall flag
    if [ "$1" = "--uninstall" ] || [ "$1" = "-u" ]; then
        uninstall
        exit 0
    fi
    
    # Run installation steps
    detect_platform
    check_requirements
    install_binary
    setup_config
    setup_claude
    setup_path
    verify_installation
}

# Run main function
main "$@"