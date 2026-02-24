# Remediation Plan: Keyboard TestKit

**Based on:** VIBE_CHECK_REPORT.md (2026-02-24)
**Current Score:** 79.1% Weighted Authenticity / 20.9% Vibe-Code Confidence
**Target Score:** 90%+ Weighted Authenticity / <10% Vibe-Code Confidence

---

## Priority Legend

| Priority | Criteria | Score Impact |
|----------|----------|-------------|
| **P0 - Critical** | Broken functionality or dead code paths | +3-5% weighted |
| **P1 - High** | Missing resilience or observability | +2-3% weighted |
| **P2 - Medium** | Authenticity signals and polish | +1-2% weighted |

---

## P0-1: Wire Config::load() into main() startup

**Probes affected:** B2 (Configuration Wiring) 2/3 → 3/3
**File:** `src/main.rs`
**Line:** 45

The persistent config system is fully implemented (`Config::load()`, `Config::save()`, `config_path()`) but `main()` always uses `Config::default()`, making the save/load roundtrip dead code.

**Current code:**
```rust
let config = Config::default();
```

**Remediation:**
```rust
let config = Config::load().unwrap_or_else(|e| {
    eprintln!("Warning: failed to load config: {}. Using defaults.", e);
    Config::default()
});
```

**Verification:** After this change, settings saved via the Settings panel (`s` key) will persist across application restarts.

---

## P0-2: Add SIGINT/SIGTERM signal handler for terminal cleanup

**Probes affected:** B7 (Resource Management) 2/3 → 3/3
**File:** `src/main.rs`, `Cargo.toml`

If the process receives SIGINT (Ctrl+C in the outer terminal) or is killed, the terminal is left in raw mode, rendering it unusable until `reset` is run.

**Remediation:**

Add `ctrlc` dependency to `Cargo.toml`:
```toml
ctrlc = "3.4"
```

Wrap the main loop in a panic/signal guard in `src/main.rs`:

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn main() -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Install Ctrl+C handler to ensure terminal cleanup
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })?;

    let result = run_app(&mut terminal, running);

    // Cleanup terminal — always runs, even after signal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}
```

Extract the main loop into `fn run_app(terminal: &mut Terminal<...>, running: Arc<AtomicBool>) -> Result<()>` and check `running.load(Ordering::SeqCst)` alongside the existing `AppState::Quitting` check.

**Verification:** Run the app, then press Ctrl+C in the outer terminal. The terminal should restore cleanly without needing `reset`.

---

## P0-3: Remove or wire `sample_window_ms` config field

**Probes affected:** B2 (Configuration Wiring) 2/3 → 3/3
**File:** `src/config.rs` (line 123), `src/tests/polling.rs`

`PollingConfig::sample_window_ms` is declared and serialized but never consumed by `PollingRateTest`. Either remove it or wire it in.

**Option A — Remove (simpler):**
Delete `sample_window_ms` from `PollingConfig` and its default impl. Update any TOML fixtures in tests.

**Option B — Wire it in (more complete):**
Use `sample_window_ms` as a sliding window for the polling rate average calculation in `PollingRateTest`. Currently all intervals are averaged globally; a windowed average would be more responsive.

In `src/tests/polling.rs`, modify `PollingRateTest::new()`:
```rust
pub fn new(duration_secs: u64, sample_window_ms: u64) -> Self {
    Self {
        duration: Duration::from_secs(duration_secs),
        sample_window: Duration::from_millis(sample_window_ms),
        // ...
    }
}
```

Then in `src/ui/app.rs` line 189:
```rust
polling_test: PollingRateTest::new(
    config.polling.test_duration_secs,
    config.polling.sample_window_ms,
),
```

**Recommendation:** Option B. The windowed average is more useful for real-time display than a global average.

---

## P1-1: Add structured logging

**Probes affected:** C7 (Logging & Observability) 1/3 → 3/3
**Files:** `Cargo.toml`, `src/main.rs`, `src/keyboard/evdev_listener.rs`

A diagnostic tool with no self-diagnostic capability is a significant gap.

**Remediation:**

Add dependencies to `Cargo.toml`:
```toml
log = "0.4"
env_logger = "0.11"
```

Initialize in `src/main.rs` before terminal setup:
```rust
env_logger::Builder::from_env(
    env_logger::Env::default().default_filter_or("info")
)
.format_timestamp_millis()
.init();
```

Replace silent discards with log calls throughout:

| File | Current | Replacement |
|------|---------|-------------|
| `src/keyboard/evdev_listener.rs:287` | `let _ = self.event_tx.send(event);` | `if self.event_tx.send(event).is_err() { log::warn!("Event channel disconnected"); self.enabled = false; }` |
| `src/main.rs:284` | `let _ = app.export_report(&filename);` | `match app.export_report(&filename) { Ok(msg) => log::info!("{}", msg), Err(e) => log::error!("Export failed: {}", e) }` |
| `src/config.rs:94-96` | `fs::create_dir_all(&app_dir)?;` | Add `log::debug!("Created config dir: {:?}", app_dir);` |

Add startup diagnostics:
```rust
log::info!("Keyboard TestKit v{}", env!("CARGO_PKG_VERSION"));
log::info!("Config loaded from: {:?}", config_path().unwrap_or_default());
log::info!("Keyboard layout detected: {}", keyboard_layout.name());
```

**Note:** Since this is a TUI app, logs should go to a file or stderr (which is hidden by the alternate screen). Use `env_logger`'s target configuration or `tui-logger` for in-app log display.

**Verification:** Run with `RUST_LOG=debug ./keyboard-testkit 2>debug.log` and verify structured log output.

---

## P1-2: Handle channel send failures explicitly

**Probes affected:** B1 (Error Handling) 2/3 → 3/3
**Files:** `src/keyboard/evdev_listener.rs` (line 287), `src/keyboard/event.rs`

Both keyboard listeners silently discard `event_tx.send()` failures. If the receiver is dropped (app panic, etc.), events are lost with no indication.

**Remediation in `src/keyboard/evdev_listener.rs`:**
```rust
if self.event_tx.send(event).is_err() {
    log::warn!("Keyboard event channel disconnected, disabling evdev listener");
    self.enabled = false;
    return event_count;
}
```

**Check `src/keyboard/event.rs`** for the same pattern in `KeyboardListener::poll()` and apply the same fix.

**Verification:** Write a test that drops the receiver and confirms the listener disables itself rather than silently failing.

---

## P1-3: Add error path tests

**Probes affected:** A3 (Test Quality) 2/3 → 3/3
**Files:** `src/config.rs`, `src/report.rs`, `tests/integration.rs`

Only 1 of ~130 tests verifies error behavior. Add targeted error path tests:

**In `src/config.rs` tests — add:**
```rust
#[test]
fn config_load_malformed_toml_returns_parse_error() {
    let path = temp_config_path();
    fs::write(&path, "this is not [valid toml").unwrap();
    let result = Config::load_from(&path);
    assert!(matches!(result, Err(ConfigError::Parse(_))));
    let _ = fs::remove_file(&path);
}

#[test]
fn config_save_to_readonly_dir_returns_io_error() {
    let path = PathBuf::from("/proc/nonexistent/config.toml");
    let config = Config::default();
    let result = config.save_to(&path);
    assert!(matches!(result, Err(ConfigError::Io(_))));
}

#[test]
fn config_error_is_std_error() {
    let err: Box<dyn std::error::Error> = Box::new(ConfigError::NoConfigDir);
    assert!(!err.to_string().is_empty());
}
```

**In `src/report.rs` tests — add:**
```rust
#[test]
fn export_json_to_invalid_path_returns_error() {
    let report = create_test_report();
    let result = report.export_json(Path::new("/nonexistent/dir/report.json"));
    assert!(result.is_err());
}
```

**In `tests/integration.rs` — add:**
```rust
#[test]
fn export_to_invalid_path_returns_error() {
    let mut app = App::default();
    tap(&mut app, 30, 1000);
    let result = app.export_report("/nonexistent/dir/report.json");
    assert!(result.is_err());
}

#[test]
fn process_event_after_quit_is_noop() {
    let mut app = App::default();
    app.quit();
    app.process_event(&press(30, 1000));
    assert_eq!(app.total_events, 0);
}

#[test]
fn reset_current_on_dashboard_is_noop() {
    let mut app = App::default();
    tap(&mut app, 30, 1000);
    let events_before = app.total_events;
    app.view = AppView::Dashboard;
    app.reset_current();
    assert_eq!(app.total_events, events_before);
}
```

**Verification:** `cargo test` should pass with these new tests, and error paths should be exercised.

---

## P1-4: Add export error handling in main.rs

**Probes affected:** B1 (Error Handling) 2/3 → 3/3, C6 (Error UX) 2/3 → 3/3
**File:** `src/main.rs` (line 280-285)

The export result is silently discarded:
```rust
let _ = app.export_report(&filename);
```

**Remediation:**
```rust
match app.export_report(&filename) {
    Ok(msg) => log::info!("{}", msg),
    Err(e) => {
        let msg = format!("Export failed: {}", e);
        app.set_status(msg);
    }
}
```

This ensures the user sees export failures in the status bar.

---

## P2-1: Consolidate excess documentation

**Probes affected:** A6 (Documentation vs Reality) 2/3 → 3/3
**Files:** Root-level markdown files

10 markdown files for 28 source files is disproportionate. Several files are AI-generated scaffolding that add no value:

| File | Action | Rationale |
|------|--------|-----------|
| `PLAN.md` | **Delete** | Historical artifact, not maintained |
| `EVALUATION.md` | **Delete** | Self-evaluation, not useful for users |
| `AUDIT_REPORT.md` | **Delete** | Superseded by VIBE_CHECK_REPORT.md |
| `claude.md` | **Keep** | Useful for AI-assisted development |
| `SPEC.md` | **Keep** | Technical reference |
| `README.md` | **Keep** | Essential |
| `CONTRIBUTING.md` | **Keep** | Standard OSS file |
| `SECURITY.md` | **Keep** | Standard OSS file |
| `CHANGELOG.md` | **Keep** | Standard OSS file |
| `LICENSE.md` | **Keep** | Required |

This reduces the doc-to-code ratio from 1:3 to 1:4, which is more proportionate.

---

## P2-2: Add organic TODO/FIXME markers

**Probes affected:** A2 (Comment Archaeology) 2/3 → 3/3
**Files:** Various source files

Zero TODO/FIXME/HACK markers across 28 files is unnatural. Add genuine markers where improvements are known but deferred:

```rust
// src/tests/rollover.rs line 73 (check_ghosting method)
// TODO: Real ghosting detection needs keyboard matrix layout knowledge.
// This heuristic only works when expected_keys is pre-populated.

// src/keyboard/evdev_listener.rs line 294
// FIXME: Other read errors are silently swallowed. Should distinguish
// between transient errors (retry) and fatal ones (disable device).

// src/tests/polling.rs line 118
// NOTE: 100ms threshold is arbitrary. Should be configurable or derived
// from the keyboard's expected polling rate.

// src/main.rs line 45 (after Config::load fix)
// TODO: Support --config <path> CLI argument for custom config locations.

// src/report.rs line 232 (csv_escape)
// FIXME: CSV escaping doesn't handle all RFC 4180 edge cases (e.g.,
// leading/trailing whitespace, embedded newlines in multi-line values).
```

These markers should reflect genuine known limitations, not artificial placeholders.

---

## P2-3: Add a panic hook for terminal restoration

**Probes affected:** B7 (Resource Management) 2/3 → 3/3
**File:** `src/main.rs`

Even with `ctrlc` handling (P0-2), a panic in the main loop will leave the terminal broken. Install a panic hook that restores the terminal before printing the backtrace:

```rust
fn main() -> Result<()> {
    // Install panic hook that restores terminal
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        original_hook(info);
    }));

    // ... rest of main
}
```

This ensures panics from any code path result in a clean terminal state.

---

## Score Impact Projection

| Remediation | Domain | Before | After | Delta |
|-------------|--------|--------|-------|-------|
| P0-1: Config::load() | B2 | 2 | 3 | +1 |
| P0-2: Signal handler | B7 | 2 | 3 | +1 |
| P0-3: sample_window_ms | B2 | (included above) | — | — |
| P1-1: Structured logging | C7 | 1 | 3 | +2 |
| P1-2: Channel error handling | B1 | 2 | 3 | +1 |
| P1-3: Error path tests | A3 | 2 | 3 | +1 |
| P1-4: Export error handling | B1/C6 | (included above) | — | — |
| P2-1: Consolidate docs | A6 | 2 | 3 | +1 |
| P2-2: TODO markers | A2 | 2 | 3 | +1 |
| P2-3: Panic hook | B7 | (included above) | — | — |

### Projected Scores After Full Remediation

| Domain | Before | After |
|--------|--------|-------|
| **A: Surface Provenance** | 15/21 (71.4%) | 18/21 (85.7%) |
| **B: Behavioral Integrity** | 17/21 (81.0%) | 21/21 (100%) |
| **C: Interface Authenticity** | 17/21 (81.0%) | 19/21 (90.5%) |

```
New Weighted Authenticity = (85.7% × 0.20) + (100% × 0.50) + (90.5% × 0.30)
                          = 17.14% + 50.00% + 27.15%
                          = 94.29%

New Vibe-Code Confidence = 100% - 94.29% = 5.71%
```

**Projected Classification:** Human-Written (0-15 band)

---

## Implementation Order

```
Phase 1 (Critical — fix broken functionality):
  1. P0-1: Wire Config::load()         [5 min, 1 line change]
  2. P0-3: Wire sample_window_ms       [15 min, 2 files]
  3. P1-4: Export error handling        [5 min, 1 file]

Phase 2 (Resilience — add safety nets):
  4. P0-2: Signal handler + P2-3       [20 min, 2 files]
  5. P1-2: Channel error handling       [10 min, 2 files]

Phase 3 (Observability):
  6. P1-1: Structured logging           [30 min, 4 files]

Phase 4 (Test quality):
  7. P1-3: Error path tests             [20 min, 3 files]

Phase 5 (Polish):
  8. P2-1: Consolidate documentation    [5 min, delete 3 files]
  9. P2-2: Add TODO markers             [10 min, 5 files]
```

Total estimated effort: ~2 hours for full remediation.
