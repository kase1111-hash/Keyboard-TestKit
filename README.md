# Keyboard TestKit

[![CI](https://github.com/kase1111-hash/Keyboard-TestKit/actions/workflows/ci.yml/badge.svg)](https://github.com/kase1111-hash/Keyboard-TestKit/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

A portable, single-executable keyboard testing and diagnostic utility with a terminal UI. Identify hardware issues, detect software conflicts, and measure keyboard performance—all from a USB drive.

## Features

| Test | Description |
|------|-------------|
| **Polling Rate** | Measure keyboard Hz (125-8000Hz) with jitter detection |
| **Stickiness** | Detect stuck or unresponsive keys |
| **Bounce** | Identify mechanical key bounce and measure hold/release timing |
| **N-Key Rollover** | Test simultaneous key capability and ghosting |
| **Latency** | Per-key and global input latency measurement |
| **Shortcuts** | Detect system hotkey conflicts intercepting input |
| **Virtual** | Compare physical vs virtual keys to isolate hardware/software issues |

## Screenshots

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ Dashboard │ Polling │ Bounce │ Sticky │ NKRO │ Latency │ Shortcuts │ Help  │
├─────────────────────────────────────────────────────────────────────────────┤
│ ⌨ Keyboard                                                                  │
│ ┌───┬───┬───┬───┬───┬───┬───┬───┬───┬───┬───┬───┬───┬───────┐             │
│ │Esc│ 1 │ 2 │ 3 │ 4 │ 5 │ 6 │ 7 │ 8 │ 9 │ 0 │ - │ = │ Bksp  │             │
│ ├───┴─┬─┴─┬─┴─┬─┴─┬─┴─┬─┴─┬─┴─┬─┴─┬─┴─┬─┴─┬─┴─┬─┴─┬─┴─┬─────┤             │
│ │ Tab │ Q │ W │[E]│ R │ T │ Y │ U │ I │ O │ P │ [ │ ] │  \  │             │
│ ├─────┴┬──┴┬──┴┬──┴┬──┴┬──┴┬──┴┬──┴┬──┴┬──┴┬──┴┬──┴┬──┴─────┤             │
│ │ Caps │[A]│[S]│[D]│ F │ G │ H │ J │ K │ L │ ; │ ' │ Enter  │             │
│ └──────┴───┴───┴───┴───┴───┴───┴───┴───┴───┴───┴───┴────────┘             │
├─────────────────────────────────────────────────────────────────────────────┤
│ Results: Dashboard                                                          │
│ ┌─────────────────────────────────────────────────────────────────────────┐ │
│ │ Session Time    12s                                                     │ │
│ │ Total Events    47                                                      │ │
│ │ Keys Pressed    4                                                       │ │
│ │ Max Rollover    4                                                       │ │
│ │ Est. Poll Rate  1000 Hz                                                 │ │
│ └─────────────────────────────────────────────────────────────────────────┘ │
├─────────────────────────────────────────────────────────────────────────────┤
│ RUNNING │ Dashboard │ 00:12 │ Events: 47 │ Tab: switch │ q: quit           │
└─────────────────────────────────────────────────────────────────────────────┘
```

*Real-time keyboard visualization with pressed keys highlighted*

## Installation

### From Source (Recommended)

**Prerequisites:**
- [Rust](https://rustup.rs/) 1.70 or later
- Linux: `libx11-dev`, `libxi-dev`, `libxtst-dev`

```bash
# Clone the repository
git clone https://github.com/kase1111-hash/Keyboard-TestKit.git
cd Keyboard-TestKit

# Build release binary
make release

# Run
./target/release/keyboard-testkit
```

### Linux Dependencies

```bash
# Debian/Ubuntu
sudo apt install libx11-dev libxi-dev libxtst-dev

# Fedora
sudo dnf install libX11-devel libXi-devel libXtst-devel

# Arch
sudo pacman -S libx11 libxi libxtst
```

### System Install (Optional)

```bash
# Install to /usr/local/bin
sudo make install

# Run from anywhere
keyboard-testkit

# Uninstall
sudo make uninstall
```

### Cross-Compile for Windows

```bash
# Install Windows target and MinGW
rustup target add x86_64-pc-windows-gnu
sudo apt install mingw-w64  # Debian/Ubuntu

# Build Windows executable
make windows
# Output: target/x86_64-pc-windows-gnu/release/keyboard-testkit.exe
```

## Usage

```bash
# Run the application
./keyboard-testkit

# Or after installation
keyboard-testkit
```

### Keyboard Controls

| Key | Action |
|-----|--------|
| `Tab` | Next test view |
| `Shift+Tab` | Previous test view |
| `1-8` | Jump to specific view |
| `m` | Toggle menu shortcuts (free number keys for testing) |
| `Space` | Pause/resume testing |
| `r` | Reset current test |
| `R` | Reset all tests |
| `e` | Export report to JSON |
| `v` | Send virtual keys (on Virtual view) |
| `?` | Show help |
| `q` / `Esc` | Quit |

### Test Views

1. **Dashboard** - Overview with session stats and real-time metrics
2. **Polling** - Keyboard polling rate (Hz) with min/max/average
3. **Bounce** - Key bounce detection and hold duration analysis
4. **Sticky** - Stuck key detection with configurable thresholds
5. **NKRO** - N-key rollover testing and ghosting detection
6. **Latency** - Per-key latency measurement in milliseconds
7. **Shortcuts** - System hotkey conflict detection
8. **Virtual** - Physical vs virtual keyboard comparison
9. **Help** - In-app help and key reference

## Configuration

Default configuration values (in `src/config.rs`):

| Setting | Default | Description |
|---------|---------|-------------|
| `polling.test_duration_secs` | 10 | Duration for polling rate test |
| `stickiness.stuck_threshold_ms` | 50 | Time before key is considered stuck |
| `hold_release.bounce_window_ms` | 5 | Window for bounce detection |
| `ui.refresh_rate_hz` | 60 | UI refresh rate |

## Diagnostic Logic

The Virtual Keyboard test helps isolate issues:

| Physical | Virtual | Diagnosis |
|----------|---------|-----------|
| Works | Works | Keyboard OK |
| Fails | Works | **Hardware Issue** |
| Fails | Fails | **Software/Driver Issue** |
| Works | Fails | API/Permission Issue |

## Export

Press `e` to export a JSON report with all test results:

```json
{
  "session_duration_secs": 120,
  "total_events": 1543,
  "polling_rate": { "average_hz": 1000, "min_hz": 995, "max_hz": 1005 },
  "rollover": { "max_keys": 6, "ghosting_detected": false },
  "latency": { "average_ms": 8.2, "per_key": {...} },
  ...
}
```

## Build Targets

```bash
make help          # Show all available targets

# Build
make release       # Optimized release binary (default)
make debug         # Debug binary with symbols
make release-full  # Build with virtual-send feature

# Test
make test          # Run tests and clippy
make check         # Check without building

# Distribution
make dist          # Create distribution package
make dist-windows  # Create Windows package
make dist-all      # Build all platforms

# Utilities
make run           # Build and run (debug)
make run-release   # Build and run (release)
make size          # Show binary size analysis
make fmt           # Format code
make doc           # Generate documentation
make clean         # Remove build artifacts
```

## Binary Size

The release binary is optimized for portability:

- **Size:** ~700-800 KB (stripped, LTO enabled)
- **Dependencies:** None (statically linked)
- **Portability:** Single executable, runs from USB drive

## Platform Support

| Platform | Status | Notes |
|----------|--------|-------|
| Linux | Full support | Primary development platform |
| Windows | Cross-compile | Requires MinGW for building |
| macOS | Cross-compile | Requires toolchain setup |

## Contributing

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

### Development

```bash
# Format code before committing
make fmt

# Run lints
cargo clippy --all-targets -- -D warnings

# Run tests
cargo test
```

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Acknowledgments

- [Ratatui](https://ratatui.rs/) - Terminal UI framework
- [device_query](https://crates.io/crates/device_query) - Cross-platform keyboard input
- [Crossterm](https://crates.io/crates/crossterm) - Terminal manipulation

---

**Keyboard TestKit** - Diagnose your keyboard, anywhere.
