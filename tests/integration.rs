//! Integration tests for Keyboard TestKit
//!
//! These tests exercise the full App pipeline: event processing through
//! all 8 test modules, state management, and report generation.

use keyboard_testkit::config::{Config, Theme};
use keyboard_testkit::keyboard::{KeyCode, KeyEvent, KeyEventType};
use keyboard_testkit::tests::KeyboardTest;
use keyboard_testkit::ui::{App, AppState, AppView};
use std::time::Instant;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn press(key: u16, delta_us: u64) -> KeyEvent {
    KeyEvent::new(KeyCode(key), KeyEventType::Press, Instant::now(), delta_us)
}

fn release(key: u16, delta_us: u64) -> KeyEvent {
    KeyEvent::new(
        KeyCode(key),
        KeyEventType::Release,
        Instant::now(),
        delta_us,
    )
}

/// Press and release a key in sequence
fn tap(app: &mut App, key: u16, delta_us: u64) {
    app.process_event(&press(key, delta_us));
    app.process_event(&release(key, delta_us));
}

/// Simulate a rapid key sequence (e.g., typing)
fn type_keys(app: &mut App, keys: &[u16], delta_us: u64) {
    for &key in keys {
        tap(app, key, delta_us);
    }
}

// ---------------------------------------------------------------------------
// Full pipeline tests
// ---------------------------------------------------------------------------

#[test]
fn full_pipeline_processes_through_all_tests() {
    let mut app = App::default();

    // Type several keys
    type_keys(&mut app, &[30, 31, 32, 33], 1000);

    assert_eq!(app.total_events, 8); // 4 press + 4 release

    // Verify each test module received events
    assert!(!app.polling_test.get_results().is_empty());
    assert!(!app.rollover_test.get_results().is_empty());
    assert!(!app.event_timing_test.get_results().is_empty());
}

#[test]
fn full_pipeline_shortcut_detection() {
    let mut app = App::default();

    // Press Ctrl+C (scancode 29=LCtrl, 46=C)
    app.process_event(&press(29, 1000));
    app.process_event(&press(46, 1000));
    app.process_event(&release(46, 1000));
    app.process_event(&release(29, 1000));

    let results = app.shortcut_test.get_results();
    let has_shortcut = results
        .iter()
        .any(|r| r.label == "Total Shortcuts" && r.value != "0");
    assert!(has_shortcut);
}

#[test]
fn full_pipeline_rollover_tracking() {
    let mut app = App::default();

    // Press 6 keys simultaneously (WASD + Space + Shift)
    let keys = [17, 30, 31, 32, 57, 42]; // W, A, S, D, Space, LShift
    for &k in &keys {
        app.process_event(&press(k, 1000));
    }

    assert_eq!(app.keyboard_state.max_rollover(), 6);
    assert_eq!(app.rollover_test.max_rollover(), 6);

    // Release all
    for &k in &keys {
        app.process_event(&release(k, 1000));
    }
    assert_eq!(app.keyboard_state.current_rollover(), 0);
}

// ---------------------------------------------------------------------------
// State management
// ---------------------------------------------------------------------------

#[test]
fn pause_resume_cycle() {
    let mut app = App::default();
    assert_eq!(app.state, AppState::Running);

    // Events processed while running
    tap(&mut app, 30, 1000);
    assert_eq!(app.total_events, 2);

    // Pause
    app.toggle_pause();
    assert_eq!(app.state, AppState::Paused);

    // Events ignored while paused
    tap(&mut app, 31, 1000);
    assert_eq!(app.total_events, 2); // unchanged

    // Resume
    app.toggle_pause();
    assert_eq!(app.state, AppState::Running);

    // Events processed again
    tap(&mut app, 32, 1000);
    assert_eq!(app.total_events, 4);
}

#[test]
fn reset_all_clears_every_test() {
    let mut app = App::default();
    type_keys(&mut app, &[30, 31, 32], 1000);
    assert!(app.total_events > 0);

    app.reset_all();

    assert_eq!(app.total_events, 0);
    assert_eq!(app.keyboard_state.current_rollover(), 0);
    assert_eq!(app.keyboard_state.max_rollover(), 0);
    assert_eq!(app.rollover_test.max_rollover(), 0);
}

#[test]
fn reset_current_only_affects_active_view() {
    let mut app = App::default();

    // Build up state in multiple tests
    type_keys(&mut app, &[30, 31], 1000);
    let events_before = app.total_events;

    // Switch to rollover view and reset
    app.view = AppView::Rollover;
    app.reset_current();

    // Rollover test reset
    assert_eq!(app.rollover_test.max_rollover(), 0);
    // But total events unchanged (that's app-level, not test-level)
    assert_eq!(app.total_events, events_before);
}

// ---------------------------------------------------------------------------
// View navigation
// ---------------------------------------------------------------------------

#[test]
fn view_navigation_cycles_all_views() {
    let mut app = App::default();
    let view_count = AppView::all().len();

    // Navigate forward through all views
    for i in 0..view_count {
        assert_eq!(app.view.index(), i);
        app.next_view();
    }
    // Should wrap back to Dashboard
    assert_eq!(app.view, AppView::Dashboard);
}

#[test]
fn view_navigation_backward_wraps() {
    let mut app = App::default();
    app.prev_view();
    assert_eq!(app.view, AppView::Help);

    app.prev_view();
    assert_eq!(app.view, AppView::OemKeys);
}

#[test]
fn view_from_index_all_views() {
    for view in AppView::all() {
        let idx = view.index();
        let roundtrip = AppView::from_index(idx);
        assert_eq!(*view, roundtrip);
    }
}

// ---------------------------------------------------------------------------
// Report generation & export
// ---------------------------------------------------------------------------

#[test]
fn report_generation_produces_valid_json() {
    let mut app = App::default();
    type_keys(&mut app, &[30, 31, 32], 1000);

    let report = app.generate_report();

    // Metadata
    assert_eq!(report.summary.total_events, 6);
    assert!(!report.metadata.generated_at.is_empty());
    assert!(!report.metadata.version.is_empty());

    // JSON roundtrip
    let json = report.to_json().expect("JSON serialization failed");
    assert!(json.contains("\"polling\""));
    assert!(json.contains("\"hold_release\""));
    assert!(json.contains("\"stickiness\""));
    assert!(json.contains("\"rollover\""));
    assert!(json.contains("\"event_timing\""));
    assert!(json.contains("\"shortcuts\""));
    assert!(json.contains("\"virtual_detect\""));
    assert!(json.contains("\"oem_keys\""));
}

#[test]
fn report_csv_export() {
    let mut app = App::default();
    type_keys(&mut app, &[30, 31], 1000);

    let report = app.generate_report();
    let csv = report.to_csv();

    // Should have header
    assert!(csv.starts_with("Category,Label,Value,Status"));
    // Should have at least some data rows
    assert!(csv.lines().count() > 1);
}

#[test]
fn report_markdown_export() {
    let mut app = App::default();
    type_keys(&mut app, &[30, 31], 1000);

    let report = app.generate_report();
    let md = report.to_markdown();

    assert!(md.contains("# Keyboard TestKit Report"));
    assert!(md.contains("## Session Information"));
    assert!(md.contains("## Summary"));
}

#[test]
fn report_text_export() {
    let mut app = App::default();
    type_keys(&mut app, &[30, 31], 1000);

    let report = app.generate_report();
    let text = report.to_text();

    assert!(text.contains("KEYBOARD TESTKIT REPORT"));
    assert!(text.contains("SUMMARY"));
    assert!(text.contains("Total Events:"));
}

#[test]
fn report_file_export_json() {
    let mut app = App::default();
    tap(&mut app, 30, 1000);

    let path = std::env::temp_dir().join(format!(
        "keyboard-testkit-test-{}.json",
        std::process::id()
    ));
    let filename = path.to_string_lossy().to_string();

    let result = app.export_report(&filename);
    assert!(result.is_ok());

    // Verify file exists and contains valid JSON
    let contents = std::fs::read_to_string(&path).expect("Failed to read exported file");
    assert!(contents.contains("\"total_events\""));

    let _ = std::fs::remove_file(&path);
}

// ---------------------------------------------------------------------------
// Configuration integration
// ---------------------------------------------------------------------------

#[test]
fn custom_config_applied_to_tests() {
    let mut config = Config::default();
    config.polling.test_duration_secs = 30;
    config.stickiness.stuck_threshold_ms = 100;
    config.hold_release.bounce_window_ms = 10;

    let app = App::new(config);

    // Config should be stored in app
    assert_eq!(app.config.polling.test_duration_secs, 30);
    assert_eq!(app.config.stickiness.stuck_threshold_ms, 100);
    assert_eq!(app.config.hold_release.bounce_window_ms, 10);
}

#[test]
fn config_theme_roundtrip() {
    let mut config = Config::default();
    assert_eq!(config.ui.theme, Theme::Dark);

    config.ui.theme = Theme::Light;

    let toml_str = toml::to_string_pretty(&config).expect("Serialize failed");
    let loaded: Config = toml::from_str(&toml_str).expect("Deserialize failed");

    assert_eq!(loaded.ui.theme, Theme::Light);
}

// ---------------------------------------------------------------------------
// Dashboard results
// ---------------------------------------------------------------------------

#[test]
fn dashboard_results_include_session_info() {
    let mut app = App::default();
    tap(&mut app, 30, 1000);

    app.view = AppView::Dashboard;
    let results = app.current_results();

    let labels: Vec<&str> = results.iter().map(|r| r.label.as_str()).collect();
    assert!(labels.contains(&"Session Time"));
    assert!(labels.contains(&"Total Events"));
    assert!(labels.contains(&"Max Rollover"));
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn no_events_produces_empty_report() {
    let app = App::default();
    let report = app.generate_report();

    assert_eq!(report.summary.total_events, 0);
    assert_eq!(report.summary.max_rollover, 0);
    assert_eq!(report.summary.issues_detected, 0);
}

#[test]
fn rapid_press_release_same_key() {
    let mut app = App::default();

    // Rapidly tap same key 50 times
    for _ in 0..50 {
        tap(&mut app, 30, 500); // 500us = 2000Hz
    }

    assert_eq!(app.total_events, 100);
    assert_eq!(app.keyboard_state.max_rollover(), 1); // Single key at a time
}

#[test]
fn high_rollover_scenario() {
    let mut app = App::default();

    // Press 10 keys simultaneously
    for key in 30..40 {
        app.process_event(&press(key, 1000));
    }

    assert_eq!(app.keyboard_state.max_rollover(), 10);
    assert_eq!(app.keyboard_state.current_rollover(), 10);

    // Release all
    for key in 30..40 {
        app.process_event(&release(key, 1000));
    }
    assert_eq!(app.keyboard_state.current_rollover(), 0);
    assert_eq!(app.keyboard_state.max_rollover(), 10); // Max preserved
}

#[test]
fn quit_state() {
    let mut app = App::default();
    app.quit();
    assert_eq!(app.state, AppState::Quitting);

    // Quitting state ignores events (paused → no processing)
    // Note: process_event checks for Running state
    app.process_event(&press(30, 1000));
    assert_eq!(app.total_events, 0);
}

#[test]
fn status_message_lifecycle() {
    let mut app = App::default();

    // Initially no status
    assert!(app.get_status().is_none());

    // Set status
    app.set_status("Test message".to_string());
    assert_eq!(app.get_status(), Some("Test message"));

    // Status should still be visible (within 3 seconds)
    assert!(app.status_message.is_some());
}

#[test]
fn toggle_shortcuts() {
    let mut app = App::default();
    assert!(app.shortcuts_enabled);

    app.toggle_shortcuts();
    assert!(!app.shortcuts_enabled);

    app.toggle_shortcuts();
    assert!(app.shortcuts_enabled);
}

#[test]
fn current_results_returns_correct_test_results() {
    let mut app = App::default();
    tap(&mut app, 30, 1000);

    // Each view should return non-panic results
    for view in AppView::all() {
        app.view = *view;
        let results = app.current_results();
        // Help view returns empty, all others should have results
        if *view != AppView::Help {
            assert!(
                !results.is_empty(),
                "View {:?} should have results",
                view
            );
        }
    }
}
