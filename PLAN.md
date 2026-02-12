# Plan: Complete Remaining Features for Keyboard TestKit

## Current State Summary

Keyboard TestKit is a portable Rust keyboard diagnostic tool at v0.1.0 with 8 working test modules, a terminal UI, TOML config, multi-format report export, 158 passing unit tests, and CI/CD. Several items from the SPEC.md and EVALUATION.md remain unfinished.

---

## Phase 1: Code Quality & Testing Infrastructure

### 1.1 Add Integration Tests for the App Struct
**Files:** new `tests/integration.rs` (top-level), `src/tests/test_helpers.rs`
**Why:** EVALUATION.md explicitly calls this out — all 158 tests are unit tests. No end-to-end tests exercise the `App` event pipeline.
**Work:**
- Create `tests/integration.rs` with integration tests that:
  - Construct an `App`, feed synthetic `KeyEvent` sequences, verify aggregated results across all 8 tests
  - Test the full pipeline: event → keyboard_state + all tests → report generation → JSON/CSV/Markdown/Text export validation
  - Test pause/resume behavior end-to-end
  - Test reset (individual + all) behavior
  - Test view navigation state machine
  - Test report export produces valid JSON, CSV, Markdown, and Text
- Expand `test_helpers.rs` with helpers for building multi-event sequences

### 1.2 Refactor App Struct to Use Dynamic Test Dispatch
**Files:** `src/ui/app.rs`, `src/tests/mod.rs`
**Why:** EVALUATION.md notes the App is a "god object" with manual dispatch to 8 tests. A `Vec<Box<dyn KeyboardTest>>` would eliminate the repetitive match statements in `process_event()`, `reset_all()`, `reset_current()`, and `current_results()`.
**Work:**
- Add a `name() -> &str` method to the `KeyboardTest` trait
- Store tests in a `Vec<Box<dyn KeyboardTest>>` inside `App`
- Replace manual dispatch loops with iteration over the test collection
- Keep named accessors (e.g., `polling_test()`) for type-specific operations like `generate_report()`
- Update existing unit tests in `app.rs`

---

## Phase 2: Light Theme Support

### 2.1 Implement Light Theme Rendering
**Files:** `src/ui/app.rs`, `src/ui/keyboard_visual.rs`, `src/ui/widgets.rs`, `src/config.rs`
**Why:** `Theme::Light` variant exists in config and serializes correctly, but the UI ignores it — all colors are hardcoded for dark backgrounds.
**Work:**
- Define a `ThemeColors` struct with named color slots (background, foreground, highlight, key_pressed, key_idle, status_bar, warning, etc.)
- Create `ThemeColors::dark()` and `ThemeColors::light()` constructors
- Pass the active `ThemeColors` to all rendering functions in `keyboard_visual.rs` and `widgets.rs`
- Replace hardcoded `Color::*` values with theme color references
- Add a runtime theme toggle hotkey (e.g., `t`)
- Add tests for theme color selection

---

## Phase 3: In-App Settings Panel

### 3.1 Add Settings View to the UI
**Files:** `src/ui/app.rs`, `src/ui/widgets.rs`, `src/config.rs`
**Why:** SPEC.md line 232 notes this is planned but not implemented. Users must edit TOML files manually.
**Work:**
- Add `AppView::Settings` variant to the view enum
- Create a settings panel widget showing editable config values:
  - `polling.test_duration_secs`
  - `stickiness.stuck_threshold_ms`
  - `hold_release.bounce_window_ms`
  - `ui.refresh_rate_hz`
  - `ui.theme` (Dark/Light)
  - `oem_keys.fn_mode`
- Add up/down navigation to select a setting, left/right or +/- to change values
- Apply changes in real-time to the running `App`
- Add a "Save to config file" action (e.g., `s` key in Settings view)
- Update the help view with settings instructions

---

## Phase 4: Shortcut Listing (Feature #6 from SPEC)

### 4.1 Enumerate System-Registered Hotkeys (Linux)
**Files:** new `src/shortcuts/system_shortcuts.rs`, `src/tests/shortcuts.rs`
**Why:** SPEC.md Feature #6 — currently the Shortcuts test only detects when system hotkeys are pressed, it does not enumerate registered global hotkeys.
**Work:**
- On Linux: parse gsettings/dconf for GNOME/KDE shortcuts, read `~/.config/` window manager configs (i3, sway, etc.), parse `/usr/share/applications/*.desktop` for defined shortcuts
- Create a `SystemShortcut` struct: `{ combo: String, application: String, action: String }`
- Add a "Known Shortcuts" section to the Shortcuts test view listing discovered system shortcuts
- Add conflict detection: highlight when a pressed combo matches a known system shortcut
- Platform-gate with `#[cfg(target_os = "linux")]`; stub on other platforms with a "not supported" message

### 4.2 Enumerate System-Registered Hotkeys (Windows)
**Files:** `src/shortcuts/system_shortcuts.rs`
**Why:** Windows support is a cross-compile target.
**Work:**
- On Windows: query RegisteredHotKey info via Windows API, scan known registry locations for application hotkeys
- Platform-gate with `#[cfg(target_os = "windows")]`

---

## Phase 5: Enhanced Shortcut Warning Indicator (Feature #7 from SPEC)

### 5.1 In-Terminal Shortcut Warning Overlay
**Files:** `src/ui/widgets.rs`, `src/ui/app.rs`
**Why:** SPEC.md Feature #7 — currently shortcut conflicts only show in the Shortcuts tab. The spec calls for a visible indicator regardless of active view.
**Work:**
- Add a notification overlay widget that appears in any view when a shortcut conflict is detected
- Display: the key combo, which known shortcut matched (if available from Phase 4), and a configurable display duration (default 2-3 seconds, from `config.ui.warning_duration_secs`)
- Render as a floating box in the top-right corner of the terminal
- Add fade-out behavior (dim colors after 1 second, disappear after duration)
- Track a history log of recent shortcut activations (last 20) accessible from the Shortcuts view

---

## Phase 6: Keyboard Layout Detection (Stretch Goal)

### 6.1 Auto-Detect Keyboard Layout
**Files:** `src/keyboard/keymap.rs`, `src/ui/keyboard_visual.rs`
**Why:** Currently hardcoded to ANSI US layout. International users see incorrect key labels in the visual.
**Work:**
- On Linux: read `setxkbmap -query` or `/etc/default/keyboard` to determine active layout
- On Windows: query `GetKeyboardLayout` API
- Support at minimum: ANSI US, ISO (UK/DE/FR), JIS
- Adjust `keyboard_visual.rs` rendering to match detected layout (key positions, labels, enter key shape)
- Fall back to ANSI US if detection fails

---

## Implementation Order & Dependencies

```
Phase 1 (Quality)     ──→  Phase 2 (Theme)    ──→  Phase 3 (Settings)
    │                                                     │
    │                                                     ▼
    └──────────────────────────────────────→  Phase 4 (Shortcut Listing)
                                                          │
                                                          ▼
                                              Phase 5 (Warning Overlay)
                                                          │
                                                          ▼
                                              Phase 6 (Layout Detection)
```

- Phase 1 should come first — refactoring the App struct makes all subsequent UI work cleaner
- Phase 2 and 3 are independent of each other but both depend on the refactored App
- Phase 4 builds on the existing shortcuts infrastructure
- Phase 5 requires Phase 4's system shortcut data to be meaningful
- Phase 6 is a stretch goal with no hard dependencies

---

## Out of Scope (Per EVALUATION.md Recommendations)

These items were explicitly recommended to CUT or DEFER:
- **OEM/FN key remapping engine removal** — The evaluation suggested cutting the remapping modes, but since they're already implemented, tested, and documented, removing them adds risk without clear benefit. Keep as-is.
- **Virtual key sending removal** — Already feature-gated behind `virtual-send`. Keep as optional.
- **CSV/Markdown/Text export removal** — Already implemented and tested. Keep as-is.
- **True end-to-end latency measurement** — Requires specialized hardware. Keep the current inter-event timing measurement which is accurately documented.
