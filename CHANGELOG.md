# Changelog

All notable changes to Keyboard TestKit will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Automatic diagnostic ("Auto" view, `A` key, `--auto` flag): a guided idle / key-sweep / hold / rollover / rapid-tap sequence that produces ranked findings (phantom input, dead keys, stuck keys, switch bounce and chatter, limited rollover, ghosting, polling rate, unknown scancodes). Findings are shown live on the Dashboard, included in exported reports under `diagnostics`, and printed on exit
- Polling-rate estimate inferred from event timestamp quantization (125/250/500 Hz grids) when evdev timestamps are available
- Key-release reporting through the kitty keyboard protocol on supporting terminals; on other terminals releases are synthesized after a short timeout so keys no longer look permanently pressed
- Shifted symbols (`!@#$%^&*()_+{}|:"<>?~`) and kitty-reported modifier keys are mapped to their physical keys instead of being dropped
- `--version` flag
- OEM key detection and remapping support
- Keyboard shortcuts for OEM/FN (9) and Auto (0) views; Help remains on `?`
- evdev-based keyboard listener for improved Linux support

### Changed
- In the test views (Polling ... OEM/FN) every key is test input except `Tab`/`Shift+Tab`, `Esc` and `Ctrl` chords, so mashing keys in the NKRO view no longer quits, resets or writes a report. Pause, reset and export are `Ctrl+P`, `Ctrl+R` and `Ctrl+E` there; single-key commands still work on the Dashboard, Auto and Help screens. In the Shortcuts view `Ctrl` combos are test input too (`Ctrl+C` still quits)
- The Timing view shows the gaps between keystrokes as information only: they measure the typist, not the keyboard, and previously rated every keyboard "Poor"
- Views, findings and reports say "n/a (terminal input)" for what terminal input cannot measure (polling rate, bounce, stuck keys, rollover peak) instead of a green "None" or ">= 1000 Hz"; `max_rollover` is `null` in exported JSON in that mode. Ghosting is reported as not measured instead of "None detected", and the virtual-keyboard check reports Inconclusive instead of verdicts it could never reach
- Bounce/chatter is a same-key re-press less than 20 ms after its release or 40 ms after its previous press, judged only from evdev kernel timestamps. The old 5 ms window could never fire on a 125 Hz keyboard; saved values are clamped to 20-40 ms
- Out-of-range values in `config.toml` are clamped on load (with a warning) instead of the whole file being discarded, and old files with the former 50 ms stuck-key default get the current default
- Updated dependencies to latest major versions: ratatui 0.29 → 0.30, crossterm 0.28 → 0.29, toml 0.8 → 1.1, enigo 0.2 → 0.6
- Refreshed Cargo.lock so all transitive dependencies are current
- Declared minimum supported Rust version (1.88) in Cargo.toml and README
- Renamed "Latency" view to "Timing": it measures the time between key events, not end-to-end input latency

### Fixed
- Auto test false alarms on healthy keyboards:
  - Holding a key for 2 s or more and releasing it (the Rollover step asks for exactly that) was a permanent Critical "Stuck key". Only a key that is still down is stuck now; long holds are informational
  - The evdev copy of the keystroke that starts the test (or of `Ctrl+N`), and the autorepeat of a held start key on terminals, were recorded as phantom input
  - The key sweep expected keys the input source cannot report (Shift/Ctrl/Alt/Win in the Windows console, modifiers in legacy terminals), so it could never pass and ended with "Dead keys". It now expects only reportable keys; keys a keyboard may lack (ISO, JIS, Right Ctrl, a locked Windows key) are warnings; only an evdev timeout is critical; a sweep still making progress is not cut off; a key jammed before the start is reported as stuck rather than dead
  - US keyboards on systems configured as `pc105` were treated as ISO and expected to press a key they do not have
  - The Rollover step accepted the first attempt with two keys; it now waits for a real attempt (6+ keys) and words a low peak as a condition
  - "Limited capture" was a warning, so terminal runs could never report no issues
- Auto test missed real faults: the chatter check could never fire (15 ms press-to-press on keyboards that report every 8 ms); one bouncing key is now reported once, and the rapid-tap rate uses the measured time
- evdev keyboards that are unplugged, replugged or reconnect over Bluetooth are picked up again (rescanned every second) instead of silently going dead; disconnects are reported as a Connection finding on the Dashboard, in reports and on exit. `SYN_DROPPED` re-reads the key state, so a release lost in a buffer overflow no longer leaves a key looking stuck
- Press-only terminals: fast same-key presses (double letters, chatter) were swallowed as repeats, holds were cut short before the autorepeat delay, and a held key could turn into a burst of presses
- Shift commands (`A`, `S`, `R`, `?`) did nothing in kitty-protocol terminals
- Keys released while paused stayed down after resuming and were later flagged as stuck
- The Bounce Window, Polling Duration and Refresh Rate settings never reached the running tests or the main loop, and `refresh_rate_hz = 0` crashed at startup
- Ctrl+Alt+Del, Ctrl+Shift+Esc and PrtSc in the known-shortcut table could never match; normal chords and switch chatter were called virtual input; laptop keys whose driver auto-releases them were counted as bounce
- CI: the workflow referenced a non-existent action (`dtolnay/rust-action`), so every job failed at setup since it was added; new clippy lints are fixed
- evdev events are now timestamped with the kernel's event time instead of the UI poll time. Previously every event in a poll batch shared one timestamp, which quantized all timing to the refresh rate and made a quick tap look like switch bounce
- Terminal key events are dispatched by kind: releases and repeats are no longer treated as new presses (on Windows every key was counted twice and controls such as Tab fired on release too), and controls only fire on the initial press
- All queued terminal events are drained each frame instead of one per frame, so bursts of keys are not spread across several redraws
- Stuck-key detection runs every frame rather than only when another key event arrives
- Default stuck-key threshold raised from 50 ms to 2 s; the old value flagged ordinary key presses as stuck
- Pressing one or two keys no longer shows the rollover result in red, and polling-view rows that reflected typing rhythm rather than the keyboard are no longer coloured as errors
- Ctrl+C now quits (raw mode previously swallowed it)
- Resolved new clippy lints (`println_empty_string`) and formatting drift so `cargo clippy -D warnings` and `cargo fmt --check` pass on current stable Rust
- Documentation now accurately describes the timing test as measuring inter-event intervals
- README export section updated to reflect all 8 tests included in JSON reports
- README keyboard controls table now documents OEM/FN view keys (a, f, c)
- SPEC.md now notes unimplemented features (shortcut listing, layout auto-detection, settings panel, shortcut overlay)
- SPEC.md dependencies list corrected to remove unused capabilities (USB enumeration, window overlay)
- Source code doc comments fixed: `LatencyTest` renamed to `EventTimingTest` with accurate description
- `lib.rs` and `claude.md` updated to reflect multi-format report export (JSON, CSV, Markdown, Text)
- EVALUATION.md updated to reflect resolved issues (report export, FnKeyMode duplication, timing labeling)
- View count updated to reflect all 10 views including OEM/FN

## [0.1.0] - 2026-01-23

### Added
- Initial release of Keyboard TestKit
- Terminal-based user interface with ratatui
- Dashboard view with session statistics
- Polling rate measurement (125-8000Hz support)
- Key bounce detection and hold duration analysis
- Stickiness (stuck key) detection
- N-Key Rollover (NKRO) testing
- Per-key inter-event timing measurement
- System shortcut conflict detection
- Virtual keyboard comparison testing
- Real-time keyboard visualization
- JSON report export functionality (polling, bounce, stickiness, rollover, and timing tests)
- Cross-platform support (Linux, Windows, macOS)
- Makefile with build targets for all platforms
- GitHub Actions CI/CD pipeline

### Technical
- Single portable executable (~700-800 KB)
- No runtime dependencies (statically linked)
- Optimized release profile with LTO
- Modular codebase architecture

[Unreleased]: https://github.com/kase1111-hash/Keyboard-TestKit/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/kase1111-hash/Keyboard-TestKit/releases/tag/v0.1.0
