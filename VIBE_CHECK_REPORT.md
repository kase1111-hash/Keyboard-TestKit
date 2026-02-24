# Vibe Check Audit Report: Keyboard TestKit

**Audit Date:** 2026-02-24
**Repository:** kase1111-hash/Keyboard-TestKit
**Auditor:** Claude Opus 4.6 (automated vibe-check v2)
**Framework:** Surface Provenance / Behavioral Integrity / Interface Authenticity

---

## Executive Summary

| Metric | Value |
|--------|-------|
| **Vibe-Code Confidence** | **20.9%** |
| **Classification** | **AI-Assisted** (16-35 band) |
| **Weighted Authenticity** | 79.1% |
| **Domain A (Surface)** | 71.4% |
| **Domain B (Behavioral)** | 81.0% |
| **Domain C (Interface)** | 81.0% |

**Verdict:** This project is transparently AI-generated (95.5% Claude-authored commits) but exhibits genuine implementation depth. The code is substantive, architecturally coherent, and functionally complete. It reads as a human-directed, AI-executed project rather than unsupervised generation. The low Vibe-Code Confidence score (20.9%) reflects that while provenance is clearly AI, the behavioral and interface quality is authentic.

---

## Domain A: Surface Provenance (71.4%)

*Weight: 20%*

### A1: Commit History — Score: 1/3 (Weak)

| Signal | Value |
|--------|-------|
| Total commits | 44 |
| AI-attributed (Claude) | 42 (95.5%) |
| Human-attributed (Kase) | 2 (4.5%) |
| Formulaic messages | 38/44 (86.4%) |
| Human frustration markers | 2 |
| Reverts | 0 |
| AI-named branches | 2 (`claude/*`) |

**Analysis:** The commit history is overwhelmingly AI-generated with no attempt to disguise it. Every commit by "Claude" follows the pattern `Verb + noun phrase` (e.g., "Add comprehensive README", "Fix clippy warnings", "Refactor App to dynamic test dispatch"). The two human commits are limited to creating LICENSE.md and the initial Readme.md. Zero reverts and near-zero frustration markers indicate a clean, directed generation session rather than iterative human development.

**Sample commits (all by Claude):**
```
7669935 Refactor App to dynamic test dispatch, add keyboard layout auto-detection
c31162a Implement remaining features: theme, settings, overlay, shortcuts, tests
e740b10 Fix all evaluation issues: latency naming, VecDeque, report export, DRY, dead code
433e01d Add OEM key detection and remapping support
e56fe4f Complete virtual keyboard testing feature
```

### A2: Comment Archaeology — Score: 2/3 (Moderate)

| Signal | Count |
|--------|-------|
| Tutorial-style comments | 0 |
| Section dividers (`====`, `----`) | 28 |
| TODO/FIXME/HACK markers | 0 |
| WHY/because comments | 12 |
| Source files | 28 |

**Analysis:** No tutorial-style comments is a positive signal. However, the complete absence of TODO/FIXME/HACK markers across 28 source files is unnatural for an actively developed project. The 28 section dividers (mostly in `report.rs`) are a mild AI pattern. The 12 WHY comments show some reasoning context but are concentrated in a few files. Comments are predominantly `//!` doc-comments (AI-typical documentation-first style) rather than inline explanations born from debugging.

**Notable WHY comments:**
```rust
// SAFETY: fcntl F_GETFL/F_SETFL are safe operations on valid file descriptors.
// Skip key repeats (value == 2)
// system_shortcuts preserved — they don't change during a session
```

### A3: Test Quality — Score: 2/3 (Moderate)

| Signal | Count |
|--------|-------|
| Test functions | ~130 |
| Trivial assertions (is_some/is_ok) | 32 |
| Error path tests | 1 |
| Parametrized tests | 0 |
| Integration tests | 22 (tests/integration.rs) |

**Analysis:** The test suite is extensive in volume but heavily happy-path. Only 1 error path test exists (`config.rs:402: assert!(result.is_err())`). Zero parametrized tests (no rstest, proptest, or quickcheck). Integration tests are well-structured and exercise the full pipeline (event → test modules → report), which shows genuine functional coverage. However, tests read as "verify the thing works" rather than "prove the thing handles adversity" — a classic AI testing pattern.

**Positive signals:**
- Integration tests verify full App pipeline, pause/resume, reset, view navigation
- Concrete value assertions (e.g., `assert!((rate - 1000.0).abs() < 0.01)`)
- Buffer limit tests (verify VecDeque capping at 100/1000)

**Negative signals:**
- No fuzzing of keyboard events or config parsing
- No test for malformed TOML input
- No test for concurrent event processing edge cases

### A4: Import & Dependency Hygiene — Score: 3/3 (Strong)

**Analysis:** All 10+ dependencies are actively used with no dead imports. Platform-conditional compilation (`#[cfg(target_os = "linux")]`) properly gates Linux-only dependencies (nix, evdev). Imports are granular (specific items, not wildcards). No vendored or duplicated dependencies.

### A5: Naming Consistency — Score: 2/3 (Moderate)

**Analysis:** Naming follows Rust conventions uniformly: PascalCase types, snake_case functions, SCREAMING_SNAKE constants. Some organic abbreviations exist (`kb_visual`, `fn_combos`, `mod_status`) but the uniformity is largely enforced by Rust's compiler and clippy. No naming anomalies or mixed conventions detected. The consistency is "too clean" but this is partially attributable to Rust's tooling rather than AI generation alone.

### A6: Documentation vs Reality — Score: 2/3 (Moderate)

| File | Purpose |
|------|---------|
| README.md | Installation, usage, features |
| SPEC.md | Technical specification |
| EVALUATION.md | Self-evaluation report |
| AUDIT_REPORT.md | Software audit |
| CONTRIBUTING.md | Contribution guidelines |
| SECURITY.md | Security policy |
| CHANGELOG.md | Version history |
| PLAN.md | Development plan |
| claude.md | AI assistant context |
| LICENSE.md | MIT license |

**Analysis:** 10 markdown documentation files for a project with 28 source files is disproportionate. The documentation-to-code ratio (~1:3) suggests AI-generated documentation scaffolding. However, the README accurately reflects the actual implementation — all 8 test types documented exist as working modules, keyboard shortcuts match the code in `main.rs`, and the ASCII mockup reflects the real UI layout. The content is accurate but the volume is suspicious.

### A7: Dependency Utilization — Score: 3/3 (Strong)

**Analysis:** Every dependency is deeply integrated:
- **ratatui**: 7 custom widget implementations (ResultsPanel, HelpPanel, StatusBar, TabBar, SettingsPanel, ShortcutOverlay, KeyboardVisual)
- **crossterm**: Terminal lifecycle management, event polling
- **device_query**: Keyboard state polling in KeyboardListener
- **nix/evdev**: Raw evdev scancode reading via `fcntl`, `read()`, `InputEvent` struct
- **serde/toml**: Full config serialization roundtrip with custom error types
- **chrono**: Report timestamp generation

No "imported but barely used" dependencies detected.

---

## Domain B: Behavioral Integrity (81.0%)

*Weight: 50%*

### B1: Error Handling Authenticity — Score: 2/3 (Moderate)

| Signal | Count |
|--------|-------|
| `.unwrap()` in non-test code | 48 |
| Custom error types | 2 (ConfigError, EvdevError) |
| `?` propagation | 38 |
| Result return types | 42 |

**Analysis:** The 48 `unwrap()` calls are concentrated in `report.rs` for `writeln!()` on String formatting — these cannot actually fail, so they're technically safe. Two custom error types (ConfigError with 4 variants, EvdevError with 4 variants) demonstrate genuine error modeling with proper `From` implementations and `Display` traits. However, channel send failures are silently discarded (`let _ = self.event_tx.send(event)` in evdev_listener.rs:287), and there is no structured logging anywhere. The error handling is competent but not battle-tested.

**ConfigError variants:**
```rust
pub enum ConfigError {
    NoConfigDir,           // Platform can't find config directory
    Io(io::Error),         // File read/write failure
    Parse(toml::de::Error), // Malformed TOML
    Serialize(toml::ser::Error), // Serialization failure
}
```

### B2: Configuration Wiring — Score: 2/3 (Moderate)

**Analysis:** Config struct is well-designed with 5 sub-configs (polling, stickiness, hold_release, ui, oem_keys). However, there are two significant gaps:

1. **Config never loaded at startup:** `main.rs:45` uses `Config::default()` — the `Config::load()` method exists but is never called. The persistent config system is implemented but not wired into the application entry point.
2. **`sample_window_ms` appears unused:** Declared in `PollingConfig` but not consumed by `PollingRateTest`.

The settings panel does allow runtime adjustment and saving, but the next launch starts from defaults again.

### B3: Call Chain Completeness — Score: 3/3 (Strong)

**Analysis:** All claimed features trace to complete implementations:
- **Polling test**: Events → `PollingRateTest::process_event()` → interval tracking → Hz calculation → results display
- **Rollover test**: Press/release tracking → simultaneous count → ghost detection → NKRO rating
- **Shortcut test**: Modifier state machine → combo detection → conflict lookup → system shortcut enumeration via gsettings
- **Virtual keyboard test**: Physical events → virtual key injection via `xdotool`/`xdg` → comparison
- **OEM/FN test**: evdev raw scancodes → FnKeyRemapper → mode-based key translation
- **Report export**: All 8 tests → ReportInput → SessionReport → JSON/CSV/Markdown/Text

No `unimplemented!()`, `todo!()`, or stub functions found. No dead modules.

### B4: Concurrency Model — Score: 3/3 (Strong)

**Analysis:** The application correctly uses a single-threaded event loop with `mpsc::channel` for keyboard event passing. No async, no thread spawning, no Mutex/RwLock. This is the appropriate model for a TUI application that polls keyboard state synchronously. The evdev listener uses non-blocking I/O (`O_NONBLOCK`) rather than threads, avoiding concurrency pitfalls entirely.

### B5: State Management — Score: 3/3 (Strong)

**Analysis:** State is properly bounded and managed:
- Per-key intervals: `VecDeque` capped at 100 entries
- Global intervals: `VecDeque` capped at 1000 entries
- Shortcut history: `VecDeque` capped at 50 entries
- Polling intervals: `Vec::with_capacity(10000)` (pre-allocated)
- `KeyboardState` has a complete `reset()` method clearing all state
- `KEYMAP` uses `LazyLock` for safe static initialization
- App state machine: Running → Paused → Quitting with proper event gating

### B6: Security Implementation — Score: 2/3 (Moderate)

| Signal | Finding |
|--------|---------|
| Hardcoded secrets | None |
| `unsafe` blocks | 2 (both in evdev_listener.rs, both with SAFETY docs) |
| Input validation | CSV escaping for export, evdev permission checks |
| Network operations | None |
| File I/O | Config save/load, report export (no path traversal protection) |

**Analysis:** The two `unsafe` blocks are well-documented and justified:
1. `fcntl()` for setting `O_NONBLOCK` on evdev file descriptors
2. `ptr::read()` for parsing kernel `input_event` structs from raw bytes

Both include `// SAFETY:` comments explaining why the operation is sound. No network surface exists. Report filenames are constructed from timestamps but user-provided filenames aren't sanitized for path traversal — low risk since the application runs locally.

### B7: Resource Management — Score: 2/3 (Moderate)

**Analysis:** No explicit `Drop` implementations, but this is appropriate:
- `File` handles in `EvdevListener` are automatically closed when dropped
- Terminal is properly restored in main (`disable_raw_mode`, `LeaveAlternateScreen`, `show_cursor`)
- No signal handler for SIGINT/SIGTERM — abnormal termination leaves terminal in raw mode (common TUI issue)
- No graceful shutdown mechanism beyond the `Quitting` state flag
- Bounded collections prevent unbounded memory growth

The terminal cleanup issue is the most significant concern — if the process is killed, the terminal may be left in an unusable state.

---

## Domain C: Interface Authenticity (81.0%)

*Weight: 30%*

### C1: API Design Consistency — Score: 3/3 (Strong)

**Analysis:** All 8 test modules implement the `KeyboardTest` trait uniformly with 5 methods (`name`, `description`, `process_event`, `is_complete`, `get_results`, `reset`). All UI widgets follow the same builder pattern (`new() → theme() → render()`). `TestResult` provides a consistent data contract with factory methods (`ok()`, `warning()`, `error()`, `info()`). The `ReportInput` struct was introduced specifically to avoid long parameter lists — a quality-of-life improvement.

### C2: UI Implementation Depth — Score: 3/3 (Strong)

**Analysis:** This is a deeply implemented TUI application:
- **7 custom widgets**: ResultsPanel, HelpPanel, StatusBar, TabBar, SettingsPanel, ShortcutOverlay, KeyboardVisual
- **10 navigable views**: Dashboard, Polling, Bounce, Sticky, NKRO, Latency, Shortcuts, Virtual, OEM/FN, Help
- **Real-time keyboard visualization** with 3 key states (unpressed, used, currently pressed) and layout detection (ANSI/ISO/JIS)
- **Theme support**: Dark/Light themes with 10 configurable colors
- **Settings panel**: Runtime configuration with arrow-key navigation and save
- **Shortcut overlay**: Floating notification box for detected shortcuts
- **Responsive rendering**: Keyboard visual checks for minimum terminal size

### C3: Frontend State Management — Score: 3/3 (Strong)

**Analysis:** The `App` struct serves as the single source of truth, owning:
- All 8 test instances
- `KeyboardState` for shared keyboard tracking
- View state (current view, settings selection)
- Application state machine (Running/Paused/Quitting)
- Config, theme, status messages

State transitions are explicit and tested (integration tests verify pause/resume, reset, view cycling, event gating).

### C4: Security Infrastructure — Score: 2/3 (Moderate)

**Analysis:** Appropriate for a local desktop TUI — no network surface, no authentication needed. Permission handling for evdev device access is properly implemented with user-friendly error messages suggesting `sudo` or `input` group membership. No unnecessary attack surface.

### C5: Real-Time Communication — Score: 3/3 (Strong)

**Analysis:** N/A for a local TUI application. The `mpsc::channel` provides appropriate intra-process communication for keyboard events.

### C6: Error UX — Score: 2/3 (Moderate)

**Analysis:** Status bar messages inform users of success/failure ("Virtual keys sent", "Virtual send failed: ..."). Evdev unavailability triggers a graceful fallback with status message. Config save errors are reported. No raw stack traces or panics exposed to users. However, status messages are transient (3-second display) with no persistent error log.

### C7: Logging & Observability — Score: 1/3 (Weak)

**Analysis:** No structured logging framework. No log levels (debug, info, warn, error). No metrics collection beyond test results. No health checks. Status messages are the only observability mechanism, and they're ephemeral. For a diagnostic tool, the irony is notable — it diagnoses keyboards thoroughly but provides no self-diagnosis capabilities.

---

## Score Calculation

### Domain A: Surface Provenance

| Probe | Score | Max |
|-------|-------|-----|
| A1: Commit History | 1 | 3 |
| A2: Comment Archaeology | 2 | 3 |
| A3: Test Quality | 2 | 3 |
| A4: Import Hygiene | 3 | 3 |
| A5: Naming Consistency | 2 | 3 |
| A6: Documentation vs Reality | 2 | 3 |
| A7: Dependency Utilization | 3 | 3 |
| **Total** | **15** | **21 (71.4%)** |

### Domain B: Behavioral Integrity

| Probe | Score | Max |
|-------|-------|-----|
| B1: Error Handling | 2 | 3 |
| B2: Configuration Wiring | 2 | 3 |
| B3: Call Chain Completeness | 3 | 3 |
| B4: Concurrency Model | 3 | 3 |
| B5: State Management | 3 | 3 |
| B6: Security Implementation | 2 | 3 |
| B7: Resource Management | 2 | 3 |
| **Total** | **17** | **21 (81.0%)** |

### Domain C: Interface Authenticity

| Probe | Score | Max |
|-------|-------|-----|
| C1: API Design Consistency | 3 | 3 |
| C2: UI Implementation Depth | 3 | 3 |
| C3: State Management | 3 | 3 |
| C4: Security Infrastructure | 2 | 3 |
| C5: Real-Time Communication | 3 | 3 |
| C6: Error UX | 2 | 3 |
| C7: Logging & Observability | 1 | 3 |
| **Total** | **17** | **21 (81.0%)** |

### Weighted Authenticity Score

```
Weighted Authenticity = (A × 0.20) + (B × 0.50) + (C × 0.30)
                      = (71.4% × 0.20) + (81.0% × 0.50) + (81.0% × 0.30)
                      = 14.28% + 40.50% + 24.30%
                      = 79.08%

Vibe-Code Confidence = 100% - 79.08% = 20.92%
```

---

## Classification

| Range | Classification | This Project |
|-------|---------------|-------------|
| 0-15 | Human-Written | |
| **16-35** | **AI-Assisted** | **20.9%** |
| 36-65 | Hybrid | |
| 66-85 | AI-Generated | |
| 86-100 | Pure Vibe-Code | |

---

## Key Findings

### Strengths
1. **Complete implementation** — All 8 claimed test modules are fully functional with real algorithms, not stubs
2. **Proper architecture** — Clean trait-based design, bounded buffers, appropriate concurrency model
3. **Deep framework usage** — 7 custom ratatui widgets, evdev raw input parsing, platform-conditional compilation
4. **Working integration tests** — Full pipeline tests exercise event processing through report generation
5. **Documented unsafe** — Both `unsafe` blocks include SAFETY comments explaining soundness

### Weaknesses
1. **Config never loaded from disk** — `Config::default()` always used at startup despite full save/load implementation
2. **Zero error path tests** — Only 1 out of ~130 tests verifies error behavior
3. **No logging** — A diagnostic tool with zero self-diagnostic capability
4. **No signal handling** — SIGINT/SIGTERM leaves terminal in raw mode
5. **Excessive documentation** — 10 markdown files for a project of this maturity is disproportionate
6. **Silent channel failures** — `let _ = self.event_tx.send(event)` discards send errors

### Recommendations
1. Wire `Config::load()` into `main()` so persistent configuration actually works
2. Add error path tests: malformed TOML, permission denied on export, channel disconnection
3. Add a `log` + `env_logger` dependency for structured logging
4. Install a `ctrlc` handler to restore the terminal on abnormal exit
5. Remove or consolidate unnecessary documentation files (PLAN.md, EVALUATION.md, AUDIT_REPORT.md)
6. Handle `event_tx.send()` failures — at minimum log them, ideally set an error flag

---

## Conclusion

Keyboard TestKit is a **transparently AI-generated project** with **genuine implementation quality**. The 95.5% Claude commit attribution makes no attempt to disguise its origin, yet the code demonstrates real understanding of keyboard diagnostics, terminal UI rendering, and systems-level Linux input handling. The Vibe-Code Confidence of 20.9% reflects that while the provenance is clearly AI, the behavioral depth exceeds typical AI-generated scaffolding. This is a functional tool, not a demonstration.

The most telling indicator of human involvement is the *specificity* of the domain — keyboard polling rate measurement, OEM scancode detection, ghosting analysis, and FN key remapping are niche requirements that suggest a knowledgeable human directing the implementation. The AI executed competently but the specification clearly came from someone who understands keyboard hardware.
