## PROJECT EVALUATION REPORT

**Primary Classification:** Full-Featured & Coherent
**Secondary Tags:** Underdeveloped (testing infrastructure gaps, latency measurement accuracy)

---

### CONCEPT ASSESSMENT

**What real problem does this solve?**
Diagnosing keyboard hardware and software issues without installing anything. Users plug in a USB drive, run a single executable, and get polling rate, stuck keys, bounce, NKRO, latency, and shortcut conflict diagnostics. This is a real need for gamers troubleshooting input lag, IT staff diagnosing laptop keyboards, and mechanical keyboard enthusiasts validating hardware.

**Who is the user?**
Three groups: (1) gamers validating polling rates and latency on competitive setups, (2) IT support diagnosing "my keyboard doesn't work right" complaints, (3) keyboard hobbyists testing switches and PCBs. The pain is real for all three — the alternative is juggling multiple web-based tools or Windows-only utilities that require installation.

**Is this solved better elsewhere?**
Partially. Windows has dedicated tools (Key Test, Keyboard Tester) but they're browser-based or require installation. Linux has `evtest` and `xev` for raw events but no unified diagnostic suite. Nothing else combines portable single-binary, cross-platform, terminal UI, and multi-test-in-one. The OEM/FN key restoration angle is genuinely novel.

**Value prop in one sentence:**
A portable, zero-install keyboard diagnostic that runs 8 hardware and software tests from a single ~700KB terminal executable.

**Verdict:** Sound — addresses a real gap with a clear differentiator (portability + comprehensiveness). The concept is well-scoped and the target user is identifiable.

---

### EXECUTION ASSESSMENT

**Architecture complexity vs actual needs:**
Appropriate. The event-driven architecture (`KeyboardListener` -> MPSC channel -> `App.process_event()` -> 8 test modules) is the natural design for this problem. The `KeyboardTest` trait (`src/tests/mod.rs:70-95`) provides clean extensibility without over-abstracting. The platform abstraction (evdev primary on Linux, device_query fallback) is the right call for a cross-platform tool.

**Code quality observations:**

*Strengths:*
- Clean trait design. The `KeyboardTest` trait at `src/tests/mod.rs:70` with `process_event()`, `get_results()`, `reset()` is well-conceived and all 8 tests implement it consistently.
- Proper Rust patterns: `Option<T>` for nullable state instead of sentinel values, `Duration`/`Instant` instead of raw integers for time, MPSC channels for thread communication.
- 143 unit tests across ~8100 LOC is a solid ratio. Tests cover edge cases like buffer limits (`src/keyboard/state.rs:276-284`), empty states, and boundary conditions.
- Release profile is well-tuned (`Cargo.toml:48-56`): `opt-level = "z"`, LTO, single codegen unit, strip — all correct for a portable binary.
- Config system with TOML serialization, platform-specific paths, and save/load roundtrip tests (`src/config.rs:393-415`) is production-quality.

*Weaknesses:*
- **Latency measurement is fundamentally limited.** `LatencyTest` at `src/tests/latency.rs:149` uses `event.delta_us` (time since last poll) as "latency." This measures poll-to-poll interval, not true input latency (physical switch actuation to software registration). The README and UI claim "input latency measurement" but the actual metric is inter-event timing. This is misleading to users.
- **O(n) removals in hot paths.** `KeyState::record_interval()` at `src/keyboard/state.rs:71` calls `Vec::remove(0)` when the buffer exceeds 100 entries. Same pattern in `KeyboardState::process_event()` at `state.rs:109`. This is O(n) on every event after warmup. Should use `VecDeque` (which `VirtualKeyboardTest` already does correctly at `virtual_detect.rs:220`). Not catastrophic at n=100/1000, but sloppy for a tool measuring microsecond-level timing.
- **`App` struct is a god object.** `src/ui/app.rs:100-135` has 15 public fields and directly owns all 8 test instances. `process_event()` at `app.rs:187-209` manually dispatches to each test. `reset_current()` at `app.rs:276-312` is a 35-line match statement. A `Vec<Box<dyn KeyboardTest>>` would eliminate the repetition, but this is a minor style issue given the fixed number of tests.
- **`SessionReport::new()` previously took 8 positional arguments and excluded 3 tests.** *(Since resolved: `SessionReport::new()` now accepts a `ReportInput` struct and includes all 8 tests in the exported report.)*
- **No integration tests.** All 143 tests are unit tests within the same module. There are no tests that exercise the `App` struct end-to-end, no tests for the main event loop, and no tests for the UI rendering path.
- **`FnKeyMode` was previously duplicated** between `config.rs` and `remap.rs`. *(Since resolved: `config.rs` now imports `FnKeyMode` from `keyboard::remap`, eliminating the duplication.)*

**Tech stack appropriateness:**
Rust is the right choice for a portable, single-binary diagnostic tool. The dependency selections are sensible: `ratatui` + `crossterm` for TUI, `device_query` for cross-platform input, `evdev` for Linux-specific raw access. No unnecessary dependencies. The `enigo` dependency is correctly feature-gated behind `virtual-send`.

**Verdict:** Execution mostly matches ambition. The architecture is clean and the code is well-organized. The latency measurement (now labeled "Timing") measures inter-event polling intervals rather than true end-to-end input latency — this is documented accurately. Report export now covers all 8 tests.

---

### SCOPE ANALYSIS

**Core Feature:** Real-time keyboard hardware diagnostics (polling rate, stuck keys, bounce, NKRO)

**Supporting:**
- Terminal UI with real-time keyboard visualization (`src/ui/keyboard_visual.rs`)
- Multi-format session export (JSON, CSV, Markdown, Text) (`src/report.rs`)
- Persistent TOML configuration (`src/config.rs`)
- Platform-adaptive input (evdev on Linux, device_query fallback)

**Nice-to-Have:**
- Shortcut conflict detection (`src/tests/shortcuts.rs`) — useful but secondary to hardware diagnostics
- Latency measurement (`src/tests/latency.rs`) — valuable in concept, but current implementation measures the wrong thing

**Distractions:**
- None. Every feature serves the core diagnostic purpose.

**Wrong Product:**
- OEM/FN key remapping (`src/keyboard/remap.rs`, `src/tests/oem_keys.rs`) — This is a keyboard configuration utility, not a diagnostic tool. Detecting OEM keys is diagnostic; *remapping* them (5 different FN modes, custom combos, persistent mappings) is a configuration tool that belongs in a separate utility. It accounts for ~600+ LOC across `remap.rs`, `oem_keys.rs`, `evdev_listener.rs`, and config additions.
- Virtual key sending (`src/tests/virtual_detect.rs:130-215`) — Detecting virtual input is diagnostic. Sending virtual keys via `enigo` to compare behavior is a test automation feature that adds significant complexity (feature flags, conditional compilation, system library dependencies) for a test that most users won't run.

**Scope Verdict:** Mostly Focused — the core diagnostic suite is well-bounded. The OEM/FN remapping feature is the main scope creep, pulling the project toward a keyboard configuration tool rather than staying in its diagnostic lane.

---

### RECOMMENDATIONS

**CUT:**
- OEM/FN key *remapping* modes (keep detection, remove `FnKeyMode::MapToFKeys`, `MapToMedia`, `RestoreWithModifier`, and the full `remap.rs` remapping engine). Detecting what OEM keys a keyboard sends is diagnostic. Remapping them is a different product.
- *(The `audio_alert` dead config field previously noted here has been removed.)*

**DEFER:**
- Virtual key sending (`virtual-send` feature) — move to a future "advanced diagnostics" version. The detection side (timing analysis, burst detection, anomaly classification) is solid and should stay.
- CSV/Markdown/Text export formats — JSON export alone is sufficient for v0.1. The other formats add ~150 LOC of maintenance surface for marginal value.
- Light theme (`config.rs:195`) — there's a `Theme::Light` variant but no implementation. Either implement it or remove the dead variant.

**DOUBLE DOWN:**
- **Latency measurement labeling.** *(Since resolved: the view has been renamed from "Latency" to "Timing" and documentation updated to describe it as measuring inter-event polling intervals rather than true end-to-end input latency.)*
- **Complete the report export.** *(Since resolved: `SessionReport` now includes all 8 tests via the `ReportInput` struct.)*
- **Replace `Vec::remove(0)` with `VecDeque`** in `KeyState::record_interval()` and `KeyboardState::process_event()`. This eliminates O(n) operations in the hot path.
- **Add integration tests.** Construct an `App`, feed it synthetic `KeyEvent` sequences, and verify aggregated results. The infrastructure already exists in `test_helpers.rs`.
- **Unify the duplicated `FnKeyMode` enum.** *(Since resolved: `config.rs` now imports `FnKeyMode` from `keyboard::remap`.)*

**FINAL VERDICT:** Continue

This is a well-conceived, well-executed project at the right scope for a v0.1 diagnostic utility. The architecture is clean, the code is idiomatic Rust, and the test coverage is respectable. Several issues identified in this evaluation have since been addressed: the timing test is now accurately labeled, report export includes all 8 tests, and `FnKeyMode` duplication is resolved. Remaining items are the `VecDeque` optimization and integration test coverage.

**Next Step:** Replace `Vec::remove(0)` with `VecDeque` in ring buffer operations and add integration tests for the `App` struct.
