# Keyboard TestKit - Build Automation
# Cross-platform Makefile for building and packaging

.PHONY: all build release debug test clean dist help install windows check-windows

# Project configuration
PROJECT := keyboard-testkit
VERSION := $(shell grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
BUILD_DIR := dist
CARGO := cargo

# Cross-compilation targets
WINDOWS_TARGET := x86_64-pc-windows-gnu

# Platform detection
ifeq ($(OS),Windows_NT)
    PLATFORM := windows
    EXE := .exe
    RM := del /Q
    MKDIR := mkdir
else
    UNAME := $(shell uname -s)
    ifeq ($(UNAME),Darwin)
        PLATFORM := macos
    else
        PLATFORM := linux
    endif
    EXE :=
    RM := rm -rf
    MKDIR := mkdir -p
endif

ARCH := $(shell uname -m)
BINARY := target/release/$(PROJECT)$(EXE)
WINDOWS_BINARY := target/$(WINDOWS_TARGET)/release/$(PROJECT).exe
DIST_NAME := $(PROJECT)-$(VERSION)-$(PLATFORM)-$(ARCH)
WINDOWS_DIST_NAME := $(PROJECT)-$(VERSION)-windows-x86_64

# Default target
all: release

# Build targets
build: release

release:
	@echo "Building release binary..."
	$(CARGO) build --release
	@echo "Done! Binary: $(BINARY)"

debug:
	@echo "Building debug binary..."
	$(CARGO) build
	@echo "Done! Binary: target/debug/$(PROJECT)$(EXE)"

# Build with virtual-send feature (requires libxdo on Linux)
release-full:
	@echo "Building with all features..."
	$(CARGO) build --release --features virtual-send
	@echo "Done!"

# Cross-compile for Windows (requires rustup target and mingw-w64)
windows:
	@echo "Cross-compiling for Windows..."
	@rustup target add $(WINDOWS_TARGET) 2>/dev/null || true
	$(CARGO) build --release --target $(WINDOWS_TARGET)
	@echo "Done! Binary: $(WINDOWS_BINARY)"

# Check Windows compilation (without linking)
check-windows:
	@echo "Checking Windows compilation..."
	@rustup target add $(WINDOWS_TARGET) 2>/dev/null || true
	$(CARGO) check --release --target $(WINDOWS_TARGET)
	@echo "Windows check passed!"

# Testing
test:
	@echo "Running tests..."
	$(CARGO) test --release
	@echo "Running clippy..."
	$(CARGO) clippy --release -- -D warnings || true
	@echo "Tests complete!"

# Check without building
check:
	$(CARGO) check --release
	$(CARGO) clippy --release

# Clean build artifacts
clean:
	$(CARGO) clean
	$(RM) $(BUILD_DIR) 2>/dev/null || true
	@echo "Cleaned!"

# Create distribution package (native)
dist: release
	@echo "Creating distribution package..."
	@$(MKDIR) $(BUILD_DIR)
	@cp $(BINARY) $(BUILD_DIR)/$(DIST_NAME)$(EXE)
ifeq ($(PLATFORM),linux)
	@strip $(BUILD_DIR)/$(DIST_NAME) 2>/dev/null || true
	@cd $(BUILD_DIR) && tar -czf $(DIST_NAME).tar.gz $(DIST_NAME)
endif
ifeq ($(PLATFORM),macos)
	@strip $(BUILD_DIR)/$(DIST_NAME) 2>/dev/null || true
	@cd $(BUILD_DIR) && tar -czf $(DIST_NAME).tar.gz $(DIST_NAME)
endif
	@echo ""
	@echo "Distribution package created:"
	@ls -lh $(BUILD_DIR)/

# Create Windows distribution package
dist-windows: windows
	@echo "Creating Windows distribution package..."
	@$(MKDIR) $(BUILD_DIR)
	@cp $(WINDOWS_BINARY) $(BUILD_DIR)/$(WINDOWS_DIST_NAME).exe
	@x86_64-w64-mingw32-strip $(BUILD_DIR)/$(WINDOWS_DIST_NAME).exe 2>/dev/null || true
	@cd $(BUILD_DIR) && zip -q $(WINDOWS_DIST_NAME).zip $(WINDOWS_DIST_NAME).exe 2>/dev/null || true
	@echo ""
	@echo "Windows distribution package created:"
	@ls -lh $(BUILD_DIR)/$(WINDOWS_DIST_NAME)*

# Build all platforms
dist-all: dist dist-windows
	@echo ""
	@echo "All distribution packages created:"
	@ls -lh $(BUILD_DIR)/

# Install to system (Linux/macOS)
install: release
ifeq ($(PLATFORM),windows)
	@echo "Manual installation required on Windows"
else
	@echo "Installing to /usr/local/bin..."
	@sudo cp $(BINARY) /usr/local/bin/$(PROJECT)
	@sudo chmod +x /usr/local/bin/$(PROJECT)
	@echo "Installed! Run with: $(PROJECT)"
endif

# Uninstall from system
uninstall:
ifeq ($(PLATFORM),windows)
	@echo "Manual uninstallation required on Windows"
else
	@sudo rm -f /usr/local/bin/$(PROJECT)
	@echo "Uninstalled!"
endif

# Development helpers
run: debug
	./target/debug/$(PROJECT)$(EXE)

run-release: release
	./$(BINARY)

# Size analysis
size: release
	@echo "Binary size analysis:"
	@ls -lh $(BINARY)
	@echo ""
	@size $(BINARY) 2>/dev/null || true

# Documentation
doc:
	$(CARGO) doc --no-deps --open

# Format code
fmt:
	$(CARGO) fmt

# Help
help:
	@echo "Keyboard TestKit Build System"
	@echo ""
	@echo "Usage: make [target]"
	@echo ""
	@echo "Build targets:"
	@echo "  release       Build optimized release binary (default)"
	@echo "  debug         Build debug binary"
	@echo "  release-full  Build with all features (virtual-send)"
	@echo "  windows       Cross-compile for Windows (requires mingw-w64)"
	@echo ""
	@echo "Test targets:"
	@echo "  test          Run all tests and clippy"
	@echo "  check         Check code without building"
	@echo "  check-windows Check Windows compilation (no linker needed)"
	@echo ""
	@echo "Distribution:"
	@echo "  dist          Create native distribution package"
	@echo "  dist-windows  Create Windows distribution package"
	@echo "  dist-all      Create all distribution packages"
	@echo "  install       Install to /usr/local/bin"
	@echo "  uninstall     Remove from /usr/local/bin"
	@echo ""
	@echo "Utility:"
	@echo "  clean         Remove build artifacts"
	@echo "  run           Build debug and run"
	@echo "  run-release   Build release and run"
	@echo "  size          Show binary size info"
	@echo "  doc           Generate documentation"
	@echo "  fmt           Format source code"
	@echo ""
	@echo "Project: $(PROJECT) v$(VERSION)"
	@echo "Platform: $(PLATFORM)-$(ARCH)"
	@echo ""
	@echo "Note: Windows cross-compilation requires:"
	@echo "  - rustup target add x86_64-pc-windows-gnu"
	@echo "  - apt install mingw-w64 (for linking)"
