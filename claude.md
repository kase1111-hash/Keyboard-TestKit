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
│   ├── oem_keys.rs      # OEM key capture & FN restoration
│   └── auto_test.rs     # Guided automatic diagnostic + findings analysis
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
3. Try evdev listener on Linux; otherwise use terminal input (kitty keyboard
   protocol for key releases where supported, synthesized releases otherwise)
4. Main loop: poll events, `app.tick()` (auto-test timers, stuck keys, live
   findings), render UI at 60Hz, drain all queued terminal events
5. Cleanup terminal on exit and print the findings summary

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
9. **AutoTest** - Guided diagnostic (idle, sweep, hold, rollover, rapid tap) whose
   `analyze()` turns the data from all tests into ranked `Finding`s. It is fed
   from `App::process_event` and driven by `App::tick`; `InputSource` tells it
   what the capture path can measure.

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
- Event timestamps come from the source (kernel time for evdev); never stamp a
  batch with the poll time, the bounce and timing tests depend on it

### UI Controls

- Tab/Shift+Tab: Navigate views
- 1-9, 0: Direct view access (0 = Auto; Help via `?`)
- A: Start the auto test (Ctrl+N skips a step, Ctrl+C aborts). While it runs,
  every other key is test input and no shortcuts fire
- Space: Pause/Resume
- r/R: Reset current/all tests
- e: Export JSON report
- q/Esc/Ctrl+C: Quit

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
