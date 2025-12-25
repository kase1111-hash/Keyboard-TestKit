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

# Build release binary
build_release() {
    local features="$1"
    local suffix=""

    header "Building Release Binary"

    if [ -n "$features" ]; then
        info "Features: $features"
        suffix="-$features"
        cargo build --release --features "$features"
    else
        info "Features: default (minimal)"
        cargo build --release
    fi

    success "Build complete"
}

# Strip and optimize binary
optimize_binary() {
    header "Optimizing Binary"

    local binary="${TARGET_DIR}/release/${PROJECT_NAME}"

    if [ "$OS" == "windows" ]; then
        binary="${binary}.exe"
    fi

    if [ ! -f "$binary" ]; then
        error "Binary not found: $binary"
    fi

    local size_before=$(du -h "$binary" | cut -f1)
    info "Size before optimization: $size_before"

    # Strip debug symbols
    if command -v strip &> /dev/null; then
        strip "$binary" 2>/dev/null || warn "Strip failed (may already be stripped)"
    fi

    local size_after=$(du -h "$binary" | cut -f1)
    success "Size after optimization: $size_after"
}

# Package for distribution
package() {
    local features="$1"
    local suffix=""

    header "Packaging for Distribution"

    mkdir -p "$BUILD_DIR"

    if [ -n "$features" ]; then
        suffix="-${features}"
    fi

    local binary_name="${PROJECT_NAME}"
    local src_binary="${TARGET_DIR}/release/${binary_name}"
    local dest_name="${PROJECT_NAME}-${VERSION}-${PLATFORM}${suffix}"

    if [ "$OS" == "windows" ]; then
        binary_name="${binary_name}.exe"
        src_binary="${src_binary}.exe"
        dest_name="${dest_name}.exe"
    fi

    # Copy binary
    cp "$src_binary" "${BUILD_DIR}/${dest_name}"
    success "Created: ${BUILD_DIR}/${dest_name}"

    # Create tarball (non-Windows)
    if [ "$OS" != "windows" ]; then
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

# Build all variants
build_all() {
    header "Building All Variants"

    # Default build (minimal, portable)
    info "Building minimal version..."
    build_release ""
    optimize_binary
    package ""

    # With virtual-send feature
    if [ "$OS" == "linux" ]; then
        if pkg-config --exists xdo 2>/dev/null; then
            info "Building with virtual-send feature..."
            build_release "virtual-send"
            optimize_binary
            package "virtual-send"
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
    echo "  all         Build all variants"
    echo "  test        Run tests then build"
    echo "  clean       Clean build artifacts"
    echo "  clean-full  Clean everything including cargo cache"
    echo "  package     Build and package for distribution"
    echo "  help        Show this help message"
    echo ""
    echo "Options:"
    echo "  --features <list>   Comma-separated features to enable"
    echo "                      Available: virtual-send"
    echo ""
    echo "Examples:"
    echo "  $0 build"
    echo "  $0 build --features virtual-send"
    echo "  $0 all"
    echo "  $0 test"
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

    # Parse arguments
    shift || true
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --features)
                features="$2"
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
            build_release "$features"
            optimize_binary
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
            build_release "$features"
            optimize_binary
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
            clean
            build_release "$features"
            optimize_binary
            package "$features"
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
