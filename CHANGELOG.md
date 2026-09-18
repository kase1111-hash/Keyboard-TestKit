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
- Updated dependencies to latest major versions: ratatui 0.29 → 0.30, crossterm 0.28 → 0.29, toml 0.8 → 1.1, enigo 0.2 → 0.6
- Refreshed Cargo.lock so all transitive dependencies are current
- Declared minimum supported Rust version (1.88) in Cargo.toml and README
- Renamed "Latency" view to "Timing" to accurately reflect that it measures inter-event polling intervals rather than true end-to-end input latency

### Fixed
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
