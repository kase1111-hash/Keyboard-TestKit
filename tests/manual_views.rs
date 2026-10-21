//! Manual test views must only show what the input source can measure, and
//! their settings must reach the running tests.

use keyboard_testkit::config::Config;
use keyboard_testkit::keyboard::{KeyCode, KeyEvent, KeyEventType};
use keyboard_testkit::tests::{InputSource, KeyboardTest, ResultStatus, Severity, TestResult};
use keyboard_testkit::ui::{App, AppView};
use std::time::{Duration, Instant};

fn at(key: u16, kind: KeyEventType, when: Instant, delta_us: u64) -> KeyEvent {
    KeyEvent::new(KeyCode(key), kind, when, delta_us)
}

/// A worn 'E' switch: one keystroke registers twice, 16 ms after the release
/// (two reports on a 125 Hz keyboard), plus 30 quick taps for the polling view.
fn feed_chattering_key_and_taps(app: &mut App) {
    let t0 = Instant::now();
    app.process_event(&at(18, KeyEventType::Press, t0, 0));
    app.process_event(&at(
        18,
        KeyEventType::Release,
        t0 + Duration::from_millis(70),
        70_000,
    ));
    app.process_event(&at(
        18,
        KeyEventType::Press,
        t0 + Duration::from_millis(86),
        16_000,
    ));
    app.process_event(&at(
        18,
        KeyEventType::Release,
        t0 + Duration::from_millis(150),
        64_000,
    ));
    let mut when = t0 + Duration::from_millis(400);
    for i in 0..30u64 {
        let gap = Duration::from_millis(8 * (4 + i % 4));
        when += gap;
        let key = 30 + (i % 2) as u16;
        app.process_event(&at(key, KeyEventType::Press, when, gap.as_micros() as u64));
        app.process_event(&at(
            key,
            KeyEventType::Release,
            when + Duration::from_millis(15),
            15_000,
        ));
    }
}

fn view(app: &mut App, view: AppView) -> Vec<TestResult> {
    app.view = view;
    app.current_results()
}

fn row(results: &[TestResult], label: &str) -> TestResult {
    results
        .iter()
        .find(|r| r.label == label)
        .unwrap_or_else(|| panic!("no '{}' row in {:?}", label, results))
        .clone()
}

#[test]
fn terminal_input_shows_not_measurable_instead_of_green() {
    for source in [
        InputSource::TerminalWithRelease,
        InputSource::TerminalPressOnly,
    ] {
        let mut app = App::default();
        app.set_input_source(source);
        feed_chattering_key_and_taps(&mut app);

        let polling = view(&mut app, AppView::PollingRate);
        let poll = row(&polling, "Est. Poll Rate");
        assert_eq!(poll.status, ResultStatus::Info, "{:?}", source);
        assert!(poll.value.contains("n/a"), "{:?}", poll);
        assert!(!polling.iter().any(|r| r.status == ResultStatus::Ok));

        let bounce = view(&mut app, AppView::HoldRelease);
        assert!(row(&bounce, "Bounces Detected").value.contains("n/a"));
        assert!(!bounce.iter().any(|r| r.status != ResultStatus::Info));
        assert_eq!(app.hold_release_test.total_bounces(), 0);

        // Nothing unmeasurable in the dashboard or the export either
        let dash = view(&mut app, AppView::Dashboard);
        assert!(!dash.iter().any(|r| r.label == "Est. Poll Rate"));
        assert!(app
            .generate_report()
            .summary
            .estimated_polling_rate_hz
            .is_none());
        app.refresh_findings();
        assert!(!app.findings.iter().any(|f| f.category == "Switch bounce"));
    }
}

#[test]
fn evdev_input_keeps_real_measurements() {
    let mut app = App::default();
    app.set_input_source(InputSource::Evdev);
    feed_chattering_key_and_taps(&mut app);

    let poll = row(&view(&mut app, AppView::PollingRate), "Est. Poll Rate");
    assert!(poll.value.starts_with("125 Hz"), "{:?}", poll);

    let bounce = view(&mut app, AppView::HoldRelease);
    assert_eq!(row(&bounce, "Bounces Detected").status, ResultStatus::Error);
    assert_eq!(app.hold_release_test.bouncy_keys(), vec![(KeyCode(18), 1)]);
    app.refresh_findings();
    assert!(app.findings.iter().any(|f| f.category == "Switch bounce"));
}

#[test]
fn evdev_clean_typing_has_no_bounce() {
    // A healthy keyboard and a normal typist, including a fast double letter
    let mut app = App::default();
    app.set_input_source(InputSource::Evdev);
    let mut when = Instant::now();
    for key in [35u16, 18, 38, 38, 24, 17, 24, 19, 38, 32] {
        app.process_event(&at(key, KeyEventType::Press, when, 0));
        app.process_event(&at(
            key,
            KeyEventType::Release,
            when + Duration::from_millis(55),
            0,
        ));
        when += Duration::from_millis(105);
    }
    assert_eq!(app.hold_release_test.total_bounces(), 0);
    let bounce = view(&mut app, AppView::HoldRelease);
    assert_eq!(row(&bounce, "Bounces Detected").status, ResultStatus::Ok);
}

#[test]
fn timing_view_never_grades_typing() {
    let mut app = App::default();
    app.set_input_source(InputSource::Evdev);
    let mut when = Instant::now();
    for (i, gap_ms) in [140u64, 95, 210, 0, 120, 300, 85].iter().enumerate() {
        when += Duration::from_millis(*gap_ms);
        app.process_event(&at(30 + i as u16, KeyEventType::Press, when, gap_ms * 1000));
    }
    let timing = view(&mut app, AppView::Latency);
    let graded: Vec<&TestResult> = timing
        .iter()
        .filter(|r| r.status != ResultStatus::Info)
        .collect();
    assert!(graded.is_empty(), "{:?}", graded);
    assert!(!timing.iter().any(|r| r.value.contains("Poor")));
}

#[test]
fn ghosting_is_reported_as_not_measured() {
    let mut app = App::default();
    app.set_input_source(InputSource::Evdev);
    let rollover = view(&mut app, AppView::Rollover);
    let ghost = row(&rollover, "Ghosting");
    assert_eq!(ghost.status, ResultStatus::Info);
    assert!(!ghost.value.contains("None"));

    // Auto test: reach the rollover step, hold some keys, stop
    app.start_auto_test();
    while app.auto_test.current_step() != Some(keyboard_testkit::tests::StepKind::Rollover) {
        app.skip_auto_step();
    }
    for key in 30..36 {
        app.process_event(&at(key, KeyEventType::Press, Instant::now(), 0));
    }
    app.abort_auto_test();
    let ghosting = app
        .findings
        .iter()
        .find(|f| f.category == "Ghosting")
        .expect("ghosting finding");
    assert_eq!(ghosting.severity, Severity::Info);
    assert!(ghosting.title.contains("Not measured"));
}

#[test]
fn virtual_check_is_unavailable_with_evdev() {
    let mut app = App::default();
    app.set_input_source(InputSource::Evdev);
    app.virtual_test.request_virtual_test();
    assert!(!app.virtual_test.has_pending_send());
    let results = view(&mut app, AppView::Virtual);
    let status = row(&results, "Status");
    assert!(status.value.starts_with("Not available"), "{:?}", status);
    // Without virtual-send the build is the reason, not evdev
    let reason = if cfg!(feature = "virtual-send") {
        "evdev"
    } else {
        "--features virtual-send"
    };
    assert!(
        results.iter().any(|r| r.value.contains(reason)),
        "{:?}",
        results
    );
}

#[test]
fn bounce_window_setting_reaches_the_test() {
    let mut config = Config::default();
    config.hold_release.bounce_window_ms = 5; // value saved by older versions
    let mut app = App::new(config);
    let item = |app: &App| {
        let items = app.settings_items();
        (items[2].label.clone(), items[2].value.clone())
    };
    assert!(item(&app).0.contains("release->press"));
    assert_eq!(item(&app).1, "20");
    assert_eq!(app.hold_release_test.bounce_window_ms(), 20);

    app.settings_selected = 2;
    app.adjust_setting(true);
    assert_eq!(app.hold_release_test.bounce_window_ms(), 21);
    assert_eq!(app.config.hold_release.bounce_window_ms, 21);
    assert_eq!(item(&app).1, "21");

    app.adjust_setting(false);
    app.adjust_setting(false);
    assert_eq!(app.hold_release_test.bounce_window_ms(), 20);
    assert_eq!(app.config.hold_release.bounce_window_ms, 20);
}

#[test]
fn polling_duration_setting_reaches_the_test() {
    let mut app = App::default();
    let before = app.polling_test.duration_secs();
    app.settings_selected = 0;
    app.adjust_setting(true);
    assert_eq!(app.polling_test.duration_secs(), before + 5);
    assert_eq!(app.config.polling.test_duration_secs, before + 5);
}

#[test]
fn shortcut_table_recognises_ctrl_alt_delete() {
    let mut app = App::default();
    for key in [29u16, 56, 111] {
        app.process_event(&at(key, KeyEventType::Press, Instant::now(), 0));
    }
    assert_eq!(app.shortcut_test.conflict_count(), 1);
    let _ = app.shortcut_test.get_results();
}
