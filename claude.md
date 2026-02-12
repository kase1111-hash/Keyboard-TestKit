# Keyboard TestKit

A portable, single-executable keyboard testing and diagnostic utility with a terminal-based UI written in Rust.

## Project Overview

Keyboard TestKit helps identify keyboard hardware issues, detect software conflicts and system hotkey interception, measure keyboard performance metrics, and provide real-time keyboard visualization. It targets keyboard enthusiasts, professionals, and support teams.

## Tech Stack

- **Language**: Rust 1.70+ (Edition 2021)
- **Terminal UI**: Ratatui 0.29 + Crossterm 0.28
- **Keyboard Input**: device_query 2.1 (cross-platform), evdev 0.12 (Linux-specific)
- **Virtual Keys**: enigo 0.2 (optional feature `virtual-send`)
- **Serialization**: serde + serde_json + toml
- **Error Handling**: anyhow + thiserror

## Project Structure

```
src/
├── main.rs              # Entry point & event loop
├── lib.rs               # Library exports
├── config.rs            # Configuration management (TOML)
├── report.rs            # Report generation (JSON, CSV, Markdown, Text)
├── utils.rs             # Utility functions
├── keyboard/            # Keyboard input handling
│   ├── event.rs         # KeyEvent types & KeyboardListener
│   ├── state.rs         # KeyboardState tracking & per-key metrics
│   ├── keymap.rs        # Key codes & keyboard layout
│   ├── remap.rs         # OEM/FN key remapping
│   └── evdev_listener.rs # Linux evdev support
├── tests/               # Test implementations
│   ├── mod.rs           # KeyboardTest trait & common structures
│   ├── polling.rs       # Polling rate measurement
│   ├── bounce.rs        # Key bounce detection
│   ├── stickiness.rs    # Stuck key detection
│   ├── rollover.rs      # NKRO testing
│   ├── latency.rs       # Inter-event timing measurement
│   ├── shortcuts.rs     # Hotkey conflict detection
│   ├── virtual_detect.rs # Physical vs virtual comparison
│   └── oem_keys.rs      # OEM key capture & FN restoration
└── ui/                  # Terminal UI components
    ├── app.rs           # Main App struct & state
    ├── keyboard_visual.rs # Real-time keyboard rendering
    └── widgets.rs       # UI widgets (ResultsPanel, TabBar, StatusBar)
```

## Key Commands

```bash
# Build
make release          # Release build (optimized for size)
make debug            # Debug build
make release-full     # With virtual-send feature

# Test & Lint
make test             # Run cargo test + clippy
make check            # Type check without building
cargo fmt             # Format code

# Run
make run              # Run debug build
make run-release      # Run release build

# Distribution
make dist             # Create distribution package
make install          # Install to /usr/local/bin
```

## Architecture

### Event Loop (main.rs)

1. Initialize terminal (raw mode, alternate screen)
2. Create keyboard listener & event channels
3. Try evdev listener on Linux (fallback to device_query)
4. Main loop: poll events, update tests, render UI at 60Hz
5. Cleanup terminal on exit

### KeyboardTest Trait

All tests implement this trait:

```rust
trait KeyboardTest {
    fn process_event(&mut self, event: &KeyEvent);  // Process key press/release
    fn get_results(&self) -> Vec<TestResult>;       // Return displayable results
    fn reset(&mut self);                            // Clear accumulated data
    fn is_complete(&self) -> bool;                  // Check if test finished
}
```

### Available Tests

1. **PollingRateTest** - Hz measurement + jitter analysis
2. **StickinessTest** - Stuck key detection
3. **RolloverTest** - NKRO testing & ghosting detection
4. **EventTimingTest** - Per-key inter-event timing (poll interval measurement)
5. **HoldReleaseTest** - Bounce detection & hold analysis
6. **ShortcutTest** - System hotkey conflict detection
7. **VirtualKeyboardTest** - Physical vs virtual key comparison
8. **OemKeyTest** - OEM/FN key capture & restoration

## Configuration

Config file locations:
- Linux: `~/.config/keyboard-testkit/config.toml`
- macOS: `~/Library/Application Support/keyboard-testkit/config.toml`
- Windows: `%APPDATA%\keyboard-testkit\config.toml`

## Conventions

### Code Patterns

- Use `anyhow::Result<T>` for recoverable errors
- Platform-specific code uses `#[cfg(target_os = "...")]`
- KeyCode uses Linux evdev scancodes as universal identifiers
- Per-key metrics tracked separately in KeyboardState

### UI Controls

- Tab/Shift+Tab: Navigate views
- 1-9, 0: Direct view access (10 views)
- Space: Pause/Resume
- r/R: Reset current/all tests
- e: Export JSON report
- q/Esc: Quit

### Build Profile

Release builds are optimized for size:
- `opt-level = "z"`
- LTO enabled
- Symbols stripped
- `panic = abort`
- Result: ~700-800 KB static executable

## Testing

Tests are inline with `#[cfg(test)]` modules. CI runs on Ubuntu, Windows, and macOS with:
- `cargo test --verbose`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo fmt --all -- --check`
