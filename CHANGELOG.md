# Changelog

All notable changes to Keyboard TestKit will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- OEM key detection and remapping support
- Keyboard shortcuts for OEM/FN (9) and Help (0) views
- evdev-based keyboard listener for improved Linux support

### Changed
- Renamed "Latency" view to "Timing" to accurately reflect that it measures inter-event polling intervals rather than true end-to-end input latency

### Fixed
- Documentation now accurately describes the timing test as measuring inter-event intervals
- Export section clarifies which tests are included in JSON reports
- SPEC.md now notes unimplemented features (layout auto-detection, settings panel, shortcut overlay)
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
