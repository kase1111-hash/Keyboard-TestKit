# Keyboard TestKit

A portable, single-executable keyboard testing and diagnostic utility with a terminal-based UI written in Rust.

## Project Overview

Keyboard TestKit helps identify keyboard hardware issues, detect software conflicts and system hotkey interception, measure keyboard performance metrics, and provide real-time keyboard visualization. It targets keyboard enthusiasts, professionals, and support teams.

## Tech Stack

- **Language**: Rust 1.88+ (Edition 2021)
- **Terminal UI**: Ratatui 0.30 + Crossterm 0.29
- **Keyboard Input**: Linux evdev read directly (libc ioctls, kernel timestamps,
  real releases); otherwise crossterm terminal events (kitty keyboard protocol
  or Windows console for releases, synthesized releases otherwise)
- **Virtual Keys**: enigo 0.6 (optional feature `virtual-send`)
- **Serialization**: serde + serde_json + toml

## Project Structure

```
src/
├── main.rs              # Entry point, event loop, key -> command mapping
├── lib.rs               # Library exports
├── mapper.rs            # Linux key-mapper daemon (--mapper)
├── config.rs            # Configuration management (TOML, clamped on load)
├── report.rs            # Report generation (JSON, CSV, Markdown, Text)
├── utils.rs             # Utility functions
├── keyboard/            # Keyboard input handling
│   ├── event.rs         # KeyEvent types & KeyboardListener
│   ├── state.rs         # KeyboardState tracking & per-key metrics
│   ├── keymap.rs        # Key codes & names
│   ├── layout.rs        # ANSI/ISO/JIS layout detection & rows
│   ├── remap.rs         # OEM/FN key remapping
│   └── evdev_listener.rs # Linux evdev support (reconnects, SYN_DROPPED, wireless keyboards)
├── tests/               # Test implementations
│   ├── mod.rs           # KeyboardTest trait & common structures
│   ├── polling.rs       # Report-rate estimate from timestamp quantization
│   ├── bounce.rs        # Hold times & switch chatter (bounce) detection
│   ├── stickiness.rs    # Stuck key detection
│   ├── rollover.rs      # NKRO counting (ghost keys shown, not detected)
│   ├── latency.rs       # Gaps between key events (informational, "Timing")
│   ├── shortcuts.rs     # Hotkey conflict detection
│   ├── virtual_detect.rs # Physical vs virtual comparison
│   ├── oem_keys.rs      # OEM key capture & FN restoration
│   └── auto_test.rs     # Guided automatic diagnostic + findings analysis
└── ui/                  # Terminal UI components
    ├── app.rs           # Main App struct & state
    ├── keyboard_visual.rs # Real-time keyboard rendering
    ├── theme.rs         # Dark/light colours
    └── widgets.rs       # UI widgets (ResultsPanel, HelpPanel, TabBar, StatusBar)
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
   protocol for key releases where supported, synthesized releases otherwise).
   The kitty flags are pushed on the alternate screen (terminals keep one
   flag stack per screen) and popped before leaving it, never with evdev.
   `App::set_input_source` tells every test what the source can measure.
4. Main loop: poll evdev (keyboards that were typed on and vanish or come
   back are reported to `App::keyboard_disconnected` / `keyboard_connected`
   and stop a running auto test; other nodes only reach the status line),
   `app.tick()` (auto-test timers, stuck keys, live findings), render
   at the configured refresh rate, drain all queued terminal events. The
   keystroke that starts the auto test is swallowed by the terminal listener
   (with its autorepeat and release) so it is never phantom input.
5. Cleanup terminal on exit (also from the panic hook: flags popped, cursor
   shown) and print the findings summary (the same `App::findings` the
   Dashboard and the exported report show). Nothing is printed to stderr
   while the TUI is drawn: messages go to the status line.

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

1. **PollingRateTest** - Report-rate estimate (125/250/500 Hz) from evdev
   timestamp quantization; key-rate and gap statistics are informational
2. **StickinessTest** - Stuck keys (still down past the threshold); released
   long holds are information only
3. **RolloverTest** - Keys registered at once, rated without modifiers
   (shown separately); ghost keys are shown, not detected
4. **EventTimingTest** - Gaps between key events: typing rhythm, never a
   keyboard rating (the poll interval comes from PollingRateTest)
5. **HoldReleaseTest** - Hold times & switch chatter (same-key re-press
   < 20-40 ms after release or < 40 ms after press, counted only with a
   hold < 20 ms on either side, after a hold >= 500 ms, or from a keyboard
   whose bus has no radio, `App::note_radio_free_keys`; evdev only;
   bounces in events from a wireless keyboard, `App::note_wireless_keys`,
   are never judged a fault)
6. **ShortcutTest** - Modifier combos and known system shortcuts (n/a when
   the source does not report modifier keys)
7. **VirtualKeyboardTest** - Physical vs virtual key comparison
8. **OemKeyTest** - OEM/FN key capture & restoration
9. **AutoTest** - Guided diagnostic (idle, sweep, hold, rollover, rapid tap) whose
   `analyze()` turns the data from all tests (plus keyboard disconnects) into
   ranked `Finding`s. It is fed from `App::process_event` and driven by
   `App::tick`; `InputSource` tells it what the capture path can measure.

## Configuration

Config file locations (`--config <path>` overrides):
- Linux: `~/.config/keyboard-testkit/config.toml` (under sudo, the invoking
  user's home, not root's; the mapper service gets the path via `--config`)
- macOS: `~/Library/Application Support/keyboard-testkit/config.toml`
- Windows: `%APPDATA%\keyboard-testkit\config.toml`

`Config::load_reporting` keeps load errors and clamped values (`ConfigLoad`)
so the app can show them; saving over a file that did not load keeps it as
`config.toml.bak`.

## Conventions

### Code Patterns

- Use `anyhow::Result<T>` for recoverable errors
- Platform-specific code uses `#[cfg(target_os = "...")]`
- KeyCode uses Linux evdev scancodes as universal identifiers
- Per-key metrics tracked separately in KeyboardState
- Event timestamps come from the source (kernel time for evdev); never stamp a
  batch with the poll time, the bounce and timing tests depend on it
- Never show a result the input source cannot measure: bounce/chatter and
  the poll-rate estimate need evdev timestamps, stuck keys and rollover need
  real releases; otherwise the view says "n/a (terminal input)" (Info) and
  the analysis skips or downgrades the check. Findings that may be the user
  rather than the keyboard (low rollover, disconnects, keystrokes during the
  idle check, a single bounce) are Warnings worded as conditions, summaries
  are red only for a Critical finding (`issues_status`), and decision logic
  lives in pure, unit-tested functions

### UI Controls

- Any view: Tab/Shift+Tab navigate, Esc/Ctrl+C quit (Esc leaves Settings),
  Ctrl+P pause, Ctrl+R reset current test, Ctrl+E export JSON report (not
  in the Shortcuts view, where Ctrl combos are test input)
- Test views (Polling ... OEM/FN): every other key is test input, except the
  view's own action keys (v on Virtual; a, f, c on OEM/FN)
- Dashboard, Auto and Help also take single keys: 1-9, 0 jump to a view,
  A starts the auto test, Space pause, r/R reset current/all, e export,
  q quit, S settings, t theme, m number keys, ? help
- While the auto test runs every key is test input except Ctrl+N (skip a
  step) and Ctrl+C (abort); after it ends plain keys stay test input until
  the user presses Enter after 2 s without typing (`App::auto_result_pending`,
  `App::auto_test_just_ended`)
- A reset (Ctrl+R, r, R) does not count its own keystroke: its keys are not
  carried over as held keys nor fed to the cleared tests
  (`App::reset_current_on_chord`). The keys are mapped from the terminal's
  character by US position (`keystroke_keys`), so with evdev on Colemak or
  Dvorak the key that typed R still counts
- A held key fires its command once, even where autorepeat arrives as
  presses (`HeldKeyFilter`); only view and Settings navigation repeats
- Key handling is decided by `command_for_key` in main.rs (unit-tested)

### Build Profile

Release builds are optimized for size:
- `opt-level = "z"`
- LTO enabled
- Symbols stripped
- `panic = abort`
- Result: ~700-800 KB static executable

## Testing

Tests are inline with `#[cfg(test)]` modules; app-level behaviour (views,
findings, reports through `App`) is tested in `tests/integration.rs` and
`tests/manual_views.rs`. CI runs on Ubuntu, Windows, and macOS with:
- `cargo test --verbose`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo fmt --all -- --check`
