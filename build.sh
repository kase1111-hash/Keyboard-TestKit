#!/usr/bin/env bash
#
# Keyboard TestKit - Build Automation Script
# Builds optimized release binaries for distribution
#

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Configuration
PROJECT_NAME="keyboard-testkit"
VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
BUILD_DIR="dist"
TARGET_DIR="target"

# Cross-compilation targets
WINDOWS_TARGET="x86_64-pc-windows-gnu"
LINUX_TARGET="x86_64-unknown-linux-gnu"
MACOS_TARGET="x86_64-apple-darwin"

# Print styled messages
info() { echo -e "${CYAN}[INFO]${NC} $1"; }
success() { echo -e "${GREEN}[OK]${NC} $1"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
error() { echo -e "${RED}[ERROR]${NC} $1"; exit 1; }
header() { echo -e "\n${BLUE}━━━ $1 ━━━${NC}\n"; }

# Detect OS and architecture
detect_platform() {
    OS=$(uname -s | tr '[:upper:]' '[:lower:]')
    ARCH=$(uname -m)

    case "$OS" in
        linux*)  OS="linux" ;;
        darwin*) OS="macos" ;;
        mingw*|msys*|cygwin*) OS="windows" ;;
        *) error "Unsupported OS: $OS" ;;
    esac

    case "$ARCH" in
        x86_64|amd64) ARCH="x86_64" ;;
        aarch64|arm64) ARCH="aarch64" ;;
        *) error "Unsupported architecture: $ARCH" ;;
    esac

    PLATFORM="${OS}-${ARCH}"
    info "Detected platform: $PLATFORM"
}

# Check dependencies
check_deps() {
    header "Checking Dependencies"

    if ! command -v cargo &> /dev/null; then
        error "Rust/Cargo not found. Install from https://rustup.rs"
    fi
    success "Cargo $(cargo --version | cut -d' ' -f2)"

    if ! command -v rustc &> /dev/null; then
        error "Rust compiler not found"
    fi
    success "Rustc $(rustc --version | cut -d' ' -f2)"
}

# Clean previous builds
clean() {
    header "Cleaning Previous Builds"

    if [ -d "$BUILD_DIR" ]; then
        rm -rf "$BUILD_DIR"
        success "Removed $BUILD_DIR/"
    fi

    if [ "$1" == "full" ]; then
        cargo clean
        success "Cleaned cargo target directory"
    fi
}

# Build release binary for a specific target
build_release() {
    local features="$1"
    local target="$2"

    header "Building Release Binary"

    local build_cmd="cargo build --release"

    if [ -n "$target" ]; then
        info "Target: $target"
        build_cmd="$build_cmd --target $target"
    fi

    if [ -n "$features" ]; then
        info "Features: $features"
        build_cmd="$build_cmd --features $features"
    else
        info "Features: default (minimal)"
    fi

    eval "$build_cmd"
    success "Build complete"
}

# Build for Windows (cross-compilation)
build_windows() {
    local features="$1"

    header "Building for Windows"

    # Check if Windows target is installed
    if ! rustup target list --installed | grep -q "$WINDOWS_TARGET"; then
        info "Installing Windows target..."
        rustup target add "$WINDOWS_TARGET"
    fi
    success "Windows target ready: $WINDOWS_TARGET"

    # Check for MinGW linker
    if ! command -v x86_64-w64-mingw32-gcc &> /dev/null; then
        warn "MinGW not found. Install with: apt install mingw-w64"
        warn "Attempting build anyway (may work with cargo's bundled linker)..."
    fi

    build_release "$features" "$WINDOWS_TARGET"
}

# Strip and optimize binary
optimize_binary() {
    local target="$1"

    header "Optimizing Binary"

    local binary
    if [ -n "$target" ]; then
        binary="${TARGET_DIR}/${target}/release/${PROJECT_NAME}"
    else
        binary="${TARGET_DIR}/release/${PROJECT_NAME}"
    fi

    # Add .exe for Windows
    if [[ "$target" == *"windows"* ]] || [ "$OS" == "windows" ]; then
        binary="${binary}.exe"
    fi

    if [ ! -f "$binary" ]; then
        error "Binary not found: $binary"
    fi

    local size_before=$(du -h "$binary" | cut -f1)
    info "Size before optimization: $size_before"

    # Strip debug symbols (use appropriate strip for target)
    if [[ "$target" == *"windows"* ]]; then
        if command -v x86_64-w64-mingw32-strip &> /dev/null; then
            x86_64-w64-mingw32-strip "$binary" 2>/dev/null || warn "Strip failed"
        else
            warn "mingw-strip not found, skipping strip for Windows binary"
        fi
    elif command -v strip &> /dev/null; then
        strip "$binary" 2>/dev/null || warn "Strip failed (may already be stripped)"
    fi

    local size_after=$(du -h "$binary" | cut -f1)
    success "Size after optimization: $size_after"
}

# Package for distribution
package() {
    local features="$1"
    local target="$2"
    local suffix=""
    local target_os="$OS"
    local target_arch="$ARCH"

    header "Packaging for Distribution"

    mkdir -p "$BUILD_DIR"

    if [ -n "$features" ]; then
        suffix="-${features}"
    fi

    # Determine platform from target
    if [ -n "$target" ]; then
        case "$target" in
            *windows*) target_os="windows"; target_arch="x86_64" ;;
            *linux*) target_os="linux"; target_arch="x86_64" ;;
            *darwin*) target_os="macos"; target_arch="x86_64" ;;
            *aarch64*) target_arch="aarch64" ;;
        esac
    fi

    local binary_name="${PROJECT_NAME}"
    local src_binary
    local dest_name="${PROJECT_NAME}-${VERSION}-${target_os}-${target_arch}${suffix}"

    if [ -n "$target" ]; then
        src_binary="${TARGET_DIR}/${target}/release/${binary_name}"
    else
        src_binary="${TARGET_DIR}/release/${binary_name}"
    fi

    if [ "$target_os" == "windows" ]; then
        binary_name="${binary_name}.exe"
        src_binary="${src_binary}.exe"
        dest_name="${dest_name}.exe"
    fi

    if [ ! -f "$src_binary" ]; then
        error "Binary not found: $src_binary"
    fi

    # Copy binary
    cp "$src_binary" "${BUILD_DIR}/${dest_name}"
    success "Created: ${BUILD_DIR}/${dest_name}"

    # Create archive
    if [ "$target_os" == "windows" ]; then
        # Create zip for Windows
        if command -v zip &> /dev/null; then
            local zipfile="${dest_name%.exe}.zip"
            (cd "$BUILD_DIR" && zip -q "$zipfile" "${dest_name}")
            success "Created: ${BUILD_DIR}/${zipfile}"
        fi
    else
        # Create tarball for Unix
        local tarball="${dest_name}.tar.gz"
        (cd "$BUILD_DIR" && tar -czf "$tarball" "$dest_name")
        success "Created: ${BUILD_DIR}/${tarball}"
    fi

    # Show final sizes
    echo ""
    info "Distribution files:"
    ls -lh "${BUILD_DIR}/"* 2>/dev/null | awk '{print "  " $9 " (" $5 ")"}'
}

# Run tests before building
run_tests() {
    header "Running Tests"

    cargo test --release
    success "All tests passed"

    cargo clippy --release -- -D warnings 2>/dev/null || warn "Clippy warnings present"
}

# Build all variants including cross-compilation
build_all() {
    header "Building All Variants"

    # Native build (minimal, portable)
    info "Building native version..."
    build_release "" ""
    optimize_binary ""
    package "" ""

    # Windows cross-compile
    info "Building Windows version..."
    build_windows ""
    optimize_binary "$WINDOWS_TARGET"
    package "" "$WINDOWS_TARGET"

    # With virtual-send feature (Linux only, requires libxdo)
    if [ "$OS" == "linux" ]; then
        if pkg-config --exists xdo 2>/dev/null; then
            info "Building with virtual-send feature..."
            build_release "virtual-send" ""
            optimize_binary ""
            package "virtual-send" ""
        else
            warn "libxdo not found, skipping virtual-send build"
        fi
    fi
}

# Print usage
usage() {
    echo "Keyboard TestKit Build Script v${VERSION}"
    echo ""
    echo "Usage: $0 [command] [options]"
    echo ""
    echo "Commands:"
    echo "  build       Build release binary (default)"
    echo "  windows     Cross-compile for Windows"
    echo "  all         Build all variants (native + Windows)"
    echo "  test        Run tests then build"
    echo "  clean       Clean build artifacts"
    echo "  clean-full  Clean everything including cargo cache"
    echo "  package     Build and package for distribution"
    echo "  help        Show this help message"
    echo ""
    echo "Options:"
    echo "  --features <list>   Comma-separated features to enable"
    echo "                      Available: virtual-send"
    echo "  --target <triple>   Cross-compile target (e.g., x86_64-pc-windows-gnu)"
    echo ""
    echo "Examples:"
    echo "  $0 build                           # Build for current platform"
    echo "  $0 windows                         # Cross-compile for Windows"
    echo "  $0 build --features virtual-send   # Build with virtual key sending"
    echo "  $0 all                             # Build all platforms"
    echo "  $0 package --target x86_64-pc-windows-gnu"
    echo ""
}

# Print build summary
summary() {
    header "Build Summary"

    echo -e "  Project:  ${CYAN}${PROJECT_NAME}${NC}"
    echo -e "  Version:  ${CYAN}${VERSION}${NC}"
    echo -e "  Platform: ${CYAN}${PLATFORM}${NC}"
    echo ""

    if [ -d "$BUILD_DIR" ]; then
        echo -e "  ${GREEN}Distribution files ready in ${BUILD_DIR}/${NC}"
        echo ""
        ls -lh "${BUILD_DIR}/"* 2>/dev/null | while read line; do
            echo "    $line"
        done
    fi

    echo ""
    success "Build completed successfully!"
}

# Main entry point
main() {
    local command="${1:-build}"
    local features=""
    local target=""

    # Parse arguments
    shift || true
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --features)
                features="$2"
                shift 2
                ;;
            --target)
                target="$2"
                shift 2
                ;;
            *)
                warn "Unknown option: $1"
                shift
                ;;
        esac
    done

    detect_platform

    case "$command" in
        build)
            check_deps
            build_release "$features" "$target"
            optimize_binary "$target"
            summary
            ;;
        windows)
            check_deps
            build_windows "$features"
            optimize_binary "$WINDOWS_TARGET"
            package "$features" "$WINDOWS_TARGET"
            summary
            ;;
        all)
            check_deps
            clean
            build_all
            summary
            ;;
        test)
            check_deps
            run_tests
            build_release "$features" "$target"
            optimize_binary "$target"
            summary
            ;;
        clean)
            clean
            success "Clean complete"
            ;;
        clean-full)
            clean "full"
            success "Full clean complete"
            ;;
        package)
            check_deps
            build_release "$features" "$target"
            optimize_binary "$target"
            package "$features" "$target"
            summary
            ;;
        help|--help|-h)
            usage
            ;;
        *)
            error "Unknown command: $command"
            ;;
    esac
}

main "$@"
