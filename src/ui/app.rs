//! Main application state and logic

use crate::config::{
    Config, Theme, BOUNCE_WINDOW_MS_MAX, BOUNCE_WINDOW_MS_MIN, POLLING_DURATION_SECS_MAX,
    POLLING_DURATION_SECS_MIN, REFRESH_RATE_HZ_MAX, REFRESH_RATE_HZ_MIN, STUCK_THRESHOLD_MS_MAX,
    STUCK_THRESHOLD_MS_MIN, WARNING_DURATION_SECS_MAX, WARNING_DURATION_SECS_MIN,
};
use crate::keyboard::layout::KeyboardLayout;
use crate::keyboard::remap::FnKeyMode;
use crate::keyboard::{KeyEvent, KeyEventType, KeyboardState};
use crate::report::{ReportInput, SessionReport};
use crate::tests::{
    expected_keys_for_layout, issues_status, AnalysisContext, AutoTest, EventTimingTest, Finding,
    HoldReleaseTest, InputSource, KeyboardTest, OemKeyTest, PollingRateTest, RolloverTest,
    ShortcutTest, StepKind, StickinessTest, TestResult, VirtualKeyboardTest,
};
use crate::ui::theme::ThemeColors;
use crate::ui::widgets::SettingsItem;
use std::path::Path;
use std::time::{Duration, Instant};

/// How often the live diagnostic findings are recomputed.
const FINDINGS_REFRESH_INTERVAL: Duration = Duration::from_millis(500);

/// Stuck-key threshold applied while the auto test asks the user to hold keys
/// (Hold and Rollover steps). Getting 6-10 keys down for the rollover step
/// takes a few seconds; 15 s leaves room for a slow attempt without flagging
/// it, while a key that really stays down is still caught once the step ends.
const HOLD_STEP_STUCK_THRESHOLD_MS: u64 = 15_000;

/// After the auto test ends (completed, or stopped), plain keys stay test
/// input until the keyboard has been quiet this long. The rapid-tap step
/// ends on a timer while the user is still tapping, and a run stopped by a
/// disconnect ends mid-sweep: the next few keys would otherwise run the Auto
/// view's single-key commands (`r` resets the run, `q` quits, Space pauses).
pub const AFTER_RUN_GRACE: Duration = Duration::from_secs(2);

/// Keyboard device nodes lost this close together are one unplug: one
/// keyboard usually has several event nodes (keys, media keys, an NKRO
/// interface) that all vanish when it is unplugged.
const DISCONNECT_MERGE_WINDOW: Duration = Duration::from_secs(2);

/// Keyboard disconnects seen since the tests were last reset.
#[derive(Debug, Default)]
struct DisconnectLog {
    /// Separate disconnects so far
    count: u32,
    /// When the last node disappeared (cleared when a keyboard comes back)
    last: Option<Instant>,
    /// Names of the devices that went away, without repeats
    devices: Vec<String>,
}

impl DisconnectLog {
    /// Record that the node of `device` disappeared at `now`.
    fn node_lost(&mut self, device: &str, now: Instant) {
        if self
            .last
            .is_none_or(|last| now.saturating_duration_since(last) >= DISCONNECT_MERGE_WINDOW)
        {
            self.count += 1;
        }
        self.last = Some(now);
        if !self.devices.iter().any(|d| d == device) {
            self.devices.push(device.to_string());
        }
    }

    /// A keyboard came back: the next node lost is a new disconnect.
    fn node_found(&mut self) {
        self.last = None;
    }
}

/// Stuck-key threshold (ms) to use during `step` of the auto test, given the
/// configured threshold. Steps that ask the user to hold keys get a relaxed
/// threshold; every other step (and no run at all) uses the configured one.
fn stuck_threshold_for_step(step: Option<StepKind>, configured_ms: u64) -> u64 {
    match step {
        Some(StepKind::Hold | StepKind::Rollover) => {
            configured_ms.max(HOLD_STEP_STUCK_THRESHOLD_MS)
        }
        _ => configured_ms,
    }
}

/// Current view/tab in the application
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppView {
    Dashboard,
    PollingRate,
    HoldRelease,
    Stickiness,
    Rollover,
    Latency,
    Shortcuts,
    Virtual,
    OemKeys,
    AutoTest,
    Help,
    Settings,
}

impl AppView {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Dashboard => "Dashboard",
            Self::PollingRate => "Polling",
            Self::HoldRelease => "Bounce",
            Self::Stickiness => "Sticky",
            Self::Rollover => "NKRO",
            Self::Latency => "Timing",
            Self::Shortcuts => "Shortcuts",
            Self::Virtual => "Virtual",
            Self::OemKeys => "OEM/FN",
            Self::AutoTest => "Auto",
            Self::Help => "Help",
            Self::Settings => "Settings",
        }
    }

    /// Views shown in the tab bar (excludes Settings which is accessed via 's')
    pub fn tab_views() -> &'static [AppView] {
        &[
            Self::Dashboard,
            Self::PollingRate,
            Self::HoldRelease,
            Self::Stickiness,
            Self::Rollover,
            Self::Latency,
            Self::Shortcuts,
            Self::Virtual,
            Self::OemKeys,
            Self::AutoTest,
            Self::Help,
        ]
    }

    pub fn all() -> &'static [AppView] {
        Self::tab_views()
    }

    pub fn index(&self) -> usize {
        match self {
            Self::Dashboard => 0,
            Self::PollingRate => 1,
            Self::HoldRelease => 2,
            Self::Stickiness => 3,
            Self::Rollover => 4,
            Self::Latency => 5,
            Self::Shortcuts => 6,
            Self::Virtual => 7,
            Self::OemKeys => 8,
            Self::AutoTest => 9,
            Self::Help => 10,
            Self::Settings => 11,
        }
    }

    pub fn from_index(index: usize) -> Self {
        match index {
            0 => Self::Dashboard,
            1 => Self::PollingRate,
            2 => Self::HoldRelease,
            3 => Self::Stickiness,
            4 => Self::Rollover,
            5 => Self::Latency,
            6 => Self::Shortcuts,
            7 => Self::Virtual,
            8 => Self::OemKeys,
            9 => Self::AutoTest,
            _ => Self::Help,
        }
    }
}

/// Application running state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppState {
    Running,
    Paused,
    Quitting,
}

/// Main application
pub struct App {
    /// Current view
    pub view: AppView,
    /// Application state
    pub state: AppState,
    /// Configuration
    pub config: Config,
    /// Keyboard state tracker
    pub keyboard_state: KeyboardState,
    /// Polling rate test
    pub polling_test: PollingRateTest,
    /// Hold and release test
    pub hold_release_test: HoldReleaseTest,
    /// Stickiness test
    pub stickiness_test: StickinessTest,
    /// Rollover test
    pub rollover_test: RolloverTest,
    /// Latency test
    pub event_timing_test: EventTimingTest,
    /// Shortcut detection test
    pub shortcut_test: ShortcutTest,
    /// Virtual keyboard detection test
    pub virtual_test: VirtualKeyboardTest,
    /// OEM key capture and FN restoration test
    pub oem_test: OemKeyTest,
    /// Application start time
    pub start_time: Instant,
    /// Total events processed
    pub total_events: u64,
    /// Last status message
    pub status_message: Option<String>,
    /// Status message timestamp
    pub status_time: Option<Instant>,
    /// Whether number key shortcuts are enabled
    pub shortcuts_enabled: bool,
    /// Current theme colors
    pub theme_colors: ThemeColors,
    /// Selected setting index in Settings view
    pub settings_selected: usize,
    /// Last detected shortcut for overlay display
    pub last_shortcut_combo: Option<String>,
    /// Last shortcut description for overlay
    pub last_shortcut_desc: Option<String>,
    /// When the last shortcut was detected (for overlay timeout)
    pub last_shortcut_time: Option<Instant>,
    /// Detected keyboard layout
    pub keyboard_layout: KeyboardLayout,
    /// Guided automatic diagnostic
    pub auto_test: AutoTest,
    /// Where key events come from (affects what the diagnostics can judge)
    pub input_source: InputSource,
    /// Latest diagnostic findings, most severe first
    pub findings: Vec<Finding>,
    /// When the findings were last recomputed
    findings_updated: Option<Instant>,
    /// Auto-test step observed on the previous tick (for step transitions)
    last_auto_step: Option<StepKind>,
    /// Keyboard disconnects since the tests were last reset
    disconnects: DisconnectLog,
    /// Latest key press the tests saw (see [`App::auto_test_just_ended`])
    last_key_press: Option<Instant>,
}

impl App {
    pub fn new(config: Config) -> Self {
        // Set up OEM test with config settings
        let mut oem_test = OemKeyTest::new();
        oem_test.set_fn_mode(config.oem_keys.fn_mode);
        oem_test.remapper_mut().set_enabled(config.oem_keys.enabled);

        // Load custom FN scancodes
        for scancode in &config.oem_keys.fn_scancodes {
            oem_test.add_fn_scancode(*scancode);
        }

        // Load custom key mappings
        for (from, to) in &config.oem_keys.key_mappings {
            oem_test.add_mapping(*from, *to);
        }

        // Load custom FN combos
        for (key, result) in &config.oem_keys.fn_combos {
            oem_test.remapper_mut().add_fn_combo(*key, *result);
        }

        let theme_colors = ThemeColors::from_theme(config.ui.theme);
        let keyboard_layout = KeyboardLayout::detect();

        // Assume the least capable source until `set_input_source` says
        // otherwise; every test is told the same thing (below)
        let input_source = InputSource::TerminalPressOnly;
        let mut auto_test = AutoTest::new(input_source);
        auto_test.set_expected_keys(expected_keys_for_layout(keyboard_layout));

        let mut app = Self {
            view: AppView::Dashboard,
            state: AppState::Running,
            config: config.clone(),
            keyboard_state: KeyboardState::new(),
            polling_test: PollingRateTest::new(
                config.polling.test_duration_secs,
                config.polling.sample_window_ms,
            ),
            hold_release_test: HoldReleaseTest::new(config.hold_release.bounce_window_ms),
            stickiness_test: StickinessTest::new(config.stickiness.stuck_threshold_ms),
            rollover_test: RolloverTest::new(),
            event_timing_test: EventTimingTest::new(),
            shortcut_test: ShortcutTest::new(),
            virtual_test: VirtualKeyboardTest::new(),
            oem_test,
            start_time: Instant::now(),
            total_events: 0,
            status_message: None,
            status_time: None,
            shortcuts_enabled: true,
            theme_colors,
            settings_selected: 0,
            last_shortcut_combo: None,
            last_shortcut_desc: None,
            last_shortcut_time: None,
            keyboard_layout,
            auto_test,
            input_source,
            findings: Vec::new(),
            findings_updated: None,
            last_auto_step: None,
            disconnects: DisconnectLog::default(),
            last_key_press: None,
        };
        app.set_input_source(input_source);
        app
    }

    /// Record that an input device of the keyboard went away (unplugged,
    /// wireless link lost). It is reported as a finding until the tests are
    /// reset or the auto test is started.
    pub fn keyboard_disconnected(&mut self, device: &str) {
        self.keyboard_disconnected_at(device, Instant::now());
    }

    fn keyboard_disconnected_at(&mut self, device: &str, now: Instant) {
        self.disconnects.node_lost(device, now);
        self.refresh_findings();
    }

    /// Record that a keyboard (re)appeared.
    pub fn keyboard_connected(&mut self) {
        self.disconnects.node_found();
    }

    /// Separate keyboard disconnects since the tests were last reset.
    pub fn keyboard_disconnects(&self) -> u32 {
        self.disconnects.count
    }

    /// Tell the app where key events come from. Call once at startup.
    pub fn set_input_source(&mut self, source: InputSource) {
        self.input_source = source;
        self.auto_test.set_input_source(source);
        // The manual views must not show results the source cannot measure
        self.polling_test.set_input_source(source);
        self.hold_release_test.set_input_source(source);
        self.stickiness_test.set_input_source(source);
        self.rollover_test.set_input_source(source);
        self.shortcut_test.set_input_source(source);
        self.virtual_test.set_input_source(source);
    }

    /// Returns mutable references to all 8 test modules for batch operations.
    fn all_tests_mut(&mut self) -> [&mut dyn KeyboardTest; 8] {
        [
            &mut self.polling_test,
            &mut self.hold_release_test,
            &mut self.stickiness_test,
            &mut self.rollover_test,
            &mut self.event_timing_test,
            &mut self.shortcut_test,
            &mut self.virtual_test,
            &mut self.oem_test,
        ]
    }

    /// Returns immutable references to all 8 test modules.
    fn all_tests(&self) -> [&dyn KeyboardTest; 8] {
        [
            &self.polling_test,
            &self.hold_release_test,
            &self.stickiness_test,
            &self.rollover_test,
            &self.event_timing_test,
            &self.shortcut_test,
            &self.virtual_test,
            &self.oem_test,
        ]
    }

    /// Map an AppView (for test views 1-8) to the corresponding test index.
    fn test_index_for_view(view: AppView) -> Option<usize> {
        match view {
            AppView::PollingRate => Some(0),
            AppView::HoldRelease => Some(1),
            AppView::Stickiness => Some(2),
            AppView::Rollover => Some(3),
            AppView::Latency => Some(4),
            AppView::Shortcuts => Some(5),
            AppView::Virtual => Some(6),
            AppView::OemKeys => Some(7),
            _ => None,
        }
    }

    /// Process a keyboard event through all active tests
    pub fn process_event(&mut self, event: &KeyEvent) {
        match self.state {
            AppState::Running => {}
            // Paused: record nothing new, but let keys that went down before
            // the pause come back up. Otherwise a key released while paused
            // (often the very key that was held when Space was pressed) would
            // look held, and later stuck, after resuming.
            AppState::Paused if self.completes_recorded_keystroke(event) => {}
            _ => return,
        }

        self.total_events += 1;
        if event.event_type == KeyEventType::Press {
            self.last_key_press = Some(
                self.last_key_press
                    .map_or(event.timestamp, |t| t.max(event.timestamp)),
            );
        }
        self.keyboard_state.process_event(event);

        // Dispatch to all tests
        for test in self.all_tests_mut() {
            test.process_event(event);
        }
        self.auto_test.process_event(event);

        // Update shortcut overlay if a new shortcut was just detected
        if let Some(last) = self.shortcut_test.recent_shortcuts(1).first() {
            if last.timestamp.elapsed().as_millis() < 100 {
                self.last_shortcut_combo = Some(last.combo.clone());
                self.last_shortcut_desc = last.description.clone();
                self.last_shortcut_time = Some(Instant::now());
            }
        }

        if self.state == AppState::Running {
            self.check_stuck_keys();
        }
    }

    /// Whether `event` is the release of a key the tests saw go down.
    fn completes_recorded_keystroke(&self, event: &KeyEvent) -> bool {
        event.event_type == KeyEventType::Release
            && self.keyboard_state.pressed_keys().contains(&event.key)
    }

    /// Flag keys held past the stickiness threshold in the status bar.
    fn check_stuck_keys(&mut self) {
        // Synthesized releases make "stuck" meaningless in press-only mode.
        if !self.input_source.has_release_events() {
            return;
        }
        let stuck = self.stickiness_test.check_stuck_keys();
        if !stuck.is_empty() {
            self.set_status(format!("Warning: {} potentially stuck key(s)", stuck.len()));
        }
    }

    /// Per-frame housekeeping: advance the auto test, re-check stuck keys and
    /// refresh the live findings. Call once per main-loop iteration.
    pub fn tick(&mut self) {
        if self.state == AppState::Running {
            self.check_stuck_keys();
        }
        // Settle a physical-vs-virtual check once its wait is over
        self.virtual_test.tick();

        let finished = self.auto_test.tick();
        self.apply_auto_step_transition();

        let stale = self
            .findings_updated
            .is_none_or(|t| t.elapsed() >= FINDINGS_REFRESH_INTERVAL);
        if finished || stale {
            self.refresh_findings();
        }
        if finished {
            self.view = AppView::AutoTest;
            self.set_completion_status();
        }
    }

    /// Say in the status line how the finished auto test went.
    fn set_completion_status(&mut self) {
        let issues = self.issue_count();
        self.set_status(if issues == 0 {
            "Auto test complete: no issues detected".to_string()
        } else {
            format!("Auto test complete: {} issue(s) found", issues)
        });
    }

    /// Relax the stuck-key threshold while the user is asked to hold keys,
    /// and restore it afterwards.
    fn apply_auto_step_transition(&mut self) {
        let step = self.auto_test.current_step();
        if step == self.last_auto_step {
            return;
        }
        self.stickiness_test.set_threshold(stuck_threshold_for_step(
            step,
            self.config.stickiness.stuck_threshold_ms,
        ));
        self.last_auto_step = step;
    }

    /// Recompute the diagnostic findings now.
    pub fn refresh_findings(&mut self) {
        self.findings = self.auto_test.analyze(&self.analysis_context());
        self.findings_updated = Some(Instant::now());
    }

    fn analysis_context(&self) -> AnalysisContext<'_> {
        AnalysisContext {
            polling: &self.polling_test,
            hold_release: &self.hold_release_test,
            stickiness: &self.stickiness_test,
            rollover: &self.rollover_test,
            event_timing: &self.event_timing_test,
            shortcuts: &self.shortcut_test,
            virtual_detect: &self.virtual_test,
            oem_keys: &self.oem_test,
            keyboard_state: &self.keyboard_state,
            keyboard_disconnects: self.disconnects.count,
            disconnected_devices: &self.disconnects.devices,
        }
    }

    /// Number of findings that count as issues (warning or critical).
    pub fn issue_count(&self) -> usize {
        self.findings.iter().filter(|f| f.is_issue()).count()
    }

    /// Presses of the keys that are down right now, with their original
    /// times. A key that is down sends no new press (evdev and the terminal
    /// listener drop autorepeat), so a reset that forgot it would hide a
    /// jammed key for good: the Sticky view would say "None detected" and a
    /// later sweep would call the key dead. Resets carry these presses over
    /// to the trackers of held keys.
    fn held_key_presses(&self) -> Vec<KeyEvent> {
        self.keyboard_state
            .pressed_keys()
            .iter()
            .filter_map(|&key| {
                let pressed_at = self.keyboard_state.get_key_state(key)?.last_press?;
                Some(KeyEvent::new(key, KeyEventType::Press, pressed_at, 0))
            })
            .collect()
    }

    /// Clear the keyboard state and every test, keeping the keys that are
    /// still down (see [`App::held_key_presses`]).
    fn reset_tests_keeping_held_keys(&mut self) {
        let held = self.held_key_presses();
        self.keyboard_state.reset();
        for test in self.all_tests_mut() {
            test.reset();
        }
        for press in &held {
            self.keyboard_state.process_event(press);
            self.stickiness_test.process_event(press);
            self.rollover_test.process_event(press);
        }
    }

    /// Start the automatic diagnostic and switch to its view.
    pub fn start_auto_test(&mut self) {
        // Give the run a clean slate so old data does not pollute the findings
        // (a keyboard replugged before the run is not a fault of the run).
        // Keys down right now (a jammed key, the Shift used to type 'A') are
        // kept: the idle step is there to catch a key stuck before the run.
        self.reset_tests_keeping_held_keys();
        self.disconnects = DisconnectLog::default();
        self.total_events = 0;
        self.stickiness_test
            .set_threshold(self.config.stickiness.stuck_threshold_ms);
        self.last_auto_step = None;
        self.auto_test.start();
        self.state = AppState::Running;
        self.view = AppView::AutoTest;
        self.refresh_findings();
        self.set_status("Auto test started - follow the instructions".to_string());
    }

    /// Stop a running automatic diagnostic, keeping what it observed.
    pub fn abort_auto_test(&mut self) {
        if self.auto_test.is_running() {
            self.auto_test.abort();
            self.apply_auto_step_transition();
            self.refresh_findings();
            self.set_status("Auto test aborted".to_string());
        }
    }

    /// Skip the current auto-test step.
    pub fn skip_auto_step(&mut self) {
        if let Some(step) = self.auto_test.current_step() {
            self.auto_test.skip_step();
            self.set_status(format!("Skipped: {}", step.title()));
            // Skipping the last step completes the run
            if self.auto_test.is_complete() {
                self.apply_auto_step_transition();
                self.refresh_findings();
                self.set_completion_status();
            }
        }
    }

    /// Whether the guided auto test is currently running.
    pub fn auto_test_running(&self) -> bool {
        self.auto_test.is_running()
    }

    /// Whether the auto test ended so recently that plain keys are still
    /// test input: less than [`AFTER_RUN_GRACE`] has passed since the run
    /// ended and since the latest key press after that.
    pub fn auto_test_just_ended(&self) -> bool {
        self.auto_test_just_ended_at(Instant::now())
    }

    fn auto_test_just_ended_at(&self, now: Instant) -> bool {
        let Some(ended) = self.auto_test.finished_at() else {
            return false;
        };
        let last = self.last_key_press.map_or(ended, |t| t.max(ended));
        now.saturating_duration_since(last) < AFTER_RUN_GRACE
    }

    /// Switch to the next view
    pub fn next_view(&mut self) {
        let views = AppView::tab_views();
        let current = self.view.index();
        let next = (current + 1) % views.len();
        self.view = AppView::from_index(next);
    }

    /// Switch to the previous view
    pub fn prev_view(&mut self) {
        let views = AppView::tab_views();
        let current = self.view.index();
        let prev = if current == 0 {
            views.len() - 1
        } else {
            current - 1
        };
        self.view = AppView::from_index(prev);
    }

    /// Toggle menu shortcuts (number keys)
    pub fn toggle_shortcuts(&mut self) {
        self.shortcuts_enabled = !self.shortcuts_enabled;
        if self.shortcuts_enabled {
            self.set_status("Menu shortcuts ON (1-9, 0)".to_string());
        } else {
            self.set_status("Menu shortcuts OFF - number keys free".to_string());
        }
    }

    /// Toggle pause state
    pub fn toggle_pause(&mut self) {
        self.state = match self.state {
            AppState::Running => {
                self.set_status("Paused".to_string());
                AppState::Paused
            }
            AppState::Paused => {
                self.set_status("Resumed".to_string());
                AppState::Running
            }
            AppState::Quitting => AppState::Quitting,
        };
    }

    /// Request quit
    pub fn quit(&mut self) {
        self.state = AppState::Quitting;
    }

    /// Reset all tests. Keys that are still down stay down.
    pub fn reset_all(&mut self) {
        self.reset_tests_keeping_held_keys();
        self.auto_test.reset();
        self.stickiness_test
            .set_threshold(self.config.stickiness.stuck_threshold_ms);
        self.last_auto_step = None;
        self.disconnects = DisconnectLog::default();
        self.total_events = 0;
        self.refresh_findings();
        self.set_status("All tests reset".to_string());
    }

    /// Reset current test
    pub fn reset_current(&mut self) {
        if self.view == AppView::AutoTest {
            self.auto_test.reset();
            // Restores the stuck-key threshold if a step had relaxed it
            self.apply_auto_step_transition();
            self.refresh_findings();
            self.set_status("Auto test reset".to_string());
            return;
        }
        if let Some(idx) = Self::test_index_for_view(self.view) {
            let name = self.all_tests()[idx].name().to_string();
            self.all_tests_mut()[idx].reset();
            // The Sticky and NKRO views track keys that are down: a jammed
            // key reset away would read "None detected" while still down
            let held = self.held_key_presses();
            for press in &held {
                match self.view {
                    AppView::Stickiness => self.stickiness_test.process_event(press),
                    AppView::Rollover => self.rollover_test.process_event(press),
                    _ => {}
                }
            }
            self.set_status(format!("{} reset", name));
        }
    }

    /// Set a status message
    pub fn set_status(&mut self, message: String) {
        self.status_message = Some(message);
        self.status_time = Some(Instant::now());
    }

    /// Get status message if still valid (within 3 seconds)
    pub fn get_status(&self) -> Option<&str> {
        match (&self.status_message, self.status_time) {
            (Some(msg), Some(time)) if time.elapsed().as_secs() < 3 => Some(msg),
            _ => None,
        }
    }

    /// Get results for current view
    pub fn current_results(&self) -> Vec<TestResult> {
        match self.view {
            AppView::Dashboard => self.dashboard_results(),
            AppView::AutoTest => self.auto_test.results(&self.findings),
            AppView::Help | AppView::Settings => Vec::new(),
            other => {
                if let Some(idx) = Self::test_index_for_view(other) {
                    self.all_tests()[idx].get_results()
                } else {
                    Vec::new()
                }
            }
        }
    }

    /// Get dashboard summary results
    fn dashboard_results(&self) -> Vec<TestResult> {
        let mut results = Vec::new();

        results.push(TestResult::info(
            "Session Time",
            format!("{:.0}s", self.start_time.elapsed().as_secs_f64()),
        ));

        results.push(TestResult::info(
            "Total Events",
            format!("{}", self.total_events),
        ));

        results.push(TestResult::info(
            "Keys Pressed",
            format!("{}", self.keyboard_state.current_rollover()),
        ));

        // Press-only terminal input guesses releases, so how many keys were
        // down at once is not known (see RolloverTest)
        results.push(TestResult::info(
            "Max Rollover",
            if self.rollover_test.rollover_measurable() {
                self.rollover_test.peak_summary()
            } else {
                "n/a (terminal input)".to_string()
            },
        ));

        if let Some(rate) = self.keyboard_state.global_polling_rate_hz() {
            results.push(TestResult::info(
                "Event Rate",
                format!("{:.1} events/s", rate),
            ));
        }
        if let Some(hz) = self.polling_test.estimated_poll_rate_hz() {
            results.push(TestResult::info("Est. Poll Rate", format!("{:.0} Hz", hz)));
        }

        results.push(TestResult::info(
            "Layout",
            self.keyboard_layout.name().to_string(),
        ));

        results.push(TestResult::info("Input", self.input_source.name()));

        // Live diagnostics summary
        results.push(TestResult::info("", ""));
        results.push(TestResult::info("--- Auto Diagnostics ---", ""));
        let issues = self.issue_count();
        // Warnings are worded as conditions (a sleeping wireless keyboard, a
        // key the laptop does not have): only a Critical finding is red
        let issue_status = issues_status(&self.findings);
        results.push(TestResult::new(
            "Issues Found",
            if self.auto_test.is_running() {
                format!("{} so far (auto test running)", issues)
            } else if self.auto_test.is_complete() {
                format!("{} (auto test complete)", issues)
            } else {
                format!("{} (press A for the full auto test)", issues)
            },
            issue_status,
        ));
        for finding in self.findings.iter().filter(|f| f.is_issue()).take(4) {
            results.push(TestResult::new(
                format!("  {}", finding.category),
                finding.title.clone(),
                finding.severity.to_status(),
            ));
        }

        results
    }

    /// Get elapsed time formatted
    pub fn elapsed_formatted(&self) -> String {
        let secs = self.start_time.elapsed().as_secs();
        let mins = secs / 60;
        let secs = secs % 60;
        format!("{:02}:{:02}", mins, secs)
    }

    /// Generate a session report
    pub fn generate_report(&self) -> SessionReport {
        SessionReport::new(ReportInput {
            start_time: self.start_time,
            total_events: self.total_events,
            max_rollover: self
                .rollover_test
                .rollover_measurable()
                .then(|| self.rollover_test.max_keys()),
            polling_rate_hz: self.polling_test.estimated_poll_rate_hz(),
            polling: self.polling_test.get_results(),
            hold_release: self.hold_release_test.get_results(),
            stickiness: self.stickiness_test.get_results(),
            rollover: self.rollover_test.get_results(),
            event_timing: self.event_timing_test.get_results(),
            shortcuts: self.shortcut_test.get_results(),
            virtual_detect: self.virtual_test.get_results(),
            oem_keys: self.oem_test.get_results(),
            diagnostics: self.findings.iter().flat_map(|f| f.to_results()).collect(),
        })
    }

    /// Export session report to JSON file
    pub fn export_report(&mut self, filename: &str) -> Result<String, std::io::Error> {
        // The live findings are only refreshed every 500 ms; the file must
        // match what was typed up to the moment of the export
        self.refresh_findings();
        let report = self.generate_report();
        let path = Path::new(filename);
        report.export_json(path)?;
        let msg = format!("Exported to {}", filename);
        self.set_status(msg.clone());
        Ok(msg)
    }

    /// Add a mapping for the last detected unknown key
    /// This allows users to remap OEM keys that are detected but not recognized
    pub fn add_oem_mapping_for_last_unknown(&mut self) {
        let unknown_keys = self.oem_test.detected_unknown();
        if unknown_keys.is_empty() {
            self.set_status("No unknown keys detected yet. Press an OEM key first.".to_string());
            return;
        }

        // Get the most recently pressed unknown key
        if let Some((&scancode, &_count)) = unknown_keys.iter().max_by_key(|(_, &c)| c) {
            // Map unknown key to itself initially (pass-through, but registered)
            // User can later configure specific mappings in config file
            self.oem_test.add_fn_scancode(scancode);
            self.set_status(format!(
                "Added scancode 0x{:03X} ({}) as FN key. Edit config for custom mapping.",
                scancode, scancode
            ));
        }
    }

    /// Cycle through FN key modes
    pub fn cycle_fn_mode(&mut self) {
        let current_mode = self.oem_test.fn_mode();
        let next_mode = match current_mode {
            FnKeyMode::Disabled => FnKeyMode::CaptureOnly,
            FnKeyMode::CaptureOnly => FnKeyMode::MapToFKeys,
            FnKeyMode::MapToFKeys => FnKeyMode::MapToMedia,
            FnKeyMode::MapToMedia => FnKeyMode::RestoreWithModifier,
            FnKeyMode::RestoreWithModifier => FnKeyMode::Disabled,
        };
        self.oem_test.set_fn_mode(next_mode);
        let mode_name = match next_mode {
            FnKeyMode::Disabled => "Disabled",
            FnKeyMode::CaptureOnly => "Capture Only",
            FnKeyMode::MapToFKeys => "Map to F-Keys",
            FnKeyMode::MapToMedia => "Map to Media",
            FnKeyMode::RestoreWithModifier => "Restore as Modifier",
        };
        self.set_status(format!("FN mode: {}", mode_name));
    }

    /// Clear all OEM key mappings
    pub fn clear_oem_mappings(&mut self) {
        self.oem_test.remapper_mut().clear_mappings();
        self.set_status("OEM key mappings cleared".to_string());
    }

    /// Toggle between dark and light theme
    pub fn toggle_theme(&mut self) {
        self.config.ui.theme = match self.config.ui.theme {
            Theme::Dark => Theme::Light,
            Theme::Light => Theme::Dark,
        };
        self.theme_colors = ThemeColors::from_theme(self.config.ui.theme);
        let name = match self.config.ui.theme {
            Theme::Dark => "Dark",
            Theme::Light => "Light",
        };
        self.set_status(format!("Theme: {}", name));
    }

    /// Build settings items for display in Settings view
    pub fn settings_items(&self) -> Vec<SettingsItem> {
        vec![
            SettingsItem {
                label: "Polling Test Duration (sec)".to_string(),
                value: format!("{}", self.config.polling.test_duration_secs),
                editable: true,
            },
            SettingsItem {
                label: "Stuck Key Threshold (ms)".to_string(),
                value: format!("{}", self.config.stickiness.stuck_threshold_ms),
                editable: true,
            },
            SettingsItem {
                // Shows the window in effect (old configs held 5 ms, raised to 20)
                label: "Bounce: release->press (ms)".to_string(),
                value: format!("{}", self.hold_release_test.bounce_window_ms()),
                editable: true,
            },
            SettingsItem {
                label: "UI Refresh Rate (Hz)".to_string(),
                value: format!("{}", self.config.ui.refresh_rate_hz),
                editable: true,
            },
            SettingsItem {
                label: "Theme".to_string(),
                value: match self.config.ui.theme {
                    Theme::Dark => "Dark".to_string(),
                    Theme::Light => "Light".to_string(),
                },
                editable: true,
            },
            SettingsItem {
                label: "FN Key Mode".to_string(),
                value: match self.oem_test.fn_mode() {
                    FnKeyMode::Disabled => "Disabled".to_string(),
                    FnKeyMode::CaptureOnly => "Capture Only".to_string(),
                    FnKeyMode::MapToFKeys => "Map to F-Keys".to_string(),
                    FnKeyMode::MapToMedia => "Map to Media".to_string(),
                    FnKeyMode::RestoreWithModifier => "Restore+Modifier".to_string(),
                },
                editable: true,
            },
            SettingsItem {
                label: "Warning Duration (sec)".to_string(),
                value: format!("{}", self.config.ui.warning_duration_secs),
                editable: true,
            },
        ]
    }

    /// Adjust the currently selected setting up or down
    pub fn adjust_setting(&mut self, increase: bool) {
        match self.settings_selected {
            0 => {
                // Polling duration
                if increase {
                    self.config.polling.test_duration_secs = self
                        .config
                        .polling
                        .test_duration_secs
                        .saturating_add(5)
                        .min(POLLING_DURATION_SECS_MAX);
                } else {
                    self.config.polling.test_duration_secs = self
                        .config
                        .polling
                        .test_duration_secs
                        .saturating_sub(5)
                        .max(POLLING_DURATION_SECS_MIN);
                }
                self.polling_test
                    .set_duration_secs(self.config.polling.test_duration_secs);
            }
            1 => {
                // Stuck threshold
                if increase {
                    self.config.stickiness.stuck_threshold_ms = self
                        .config
                        .stickiness
                        .stuck_threshold_ms
                        .saturating_add(250)
                        .min(STUCK_THRESHOLD_MS_MAX);
                } else {
                    self.config.stickiness.stuck_threshold_ms = self
                        .config
                        .stickiness
                        .stuck_threshold_ms
                        .saturating_sub(250)
                        .max(STUCK_THRESHOLD_MS_MIN);
                }
                self.stickiness_test
                    .set_threshold(self.config.stickiness.stuck_threshold_ms);
            }
            2 => {
                // Bounce window, stepped from the value in effect (an old
                // config's 5 ms runs, and is shown, as 20)
                let current = self.hold_release_test.bounce_window_ms();
                let next = if increase {
                    current.saturating_add(1)
                } else {
                    current.saturating_sub(1)
                };
                self.config.hold_release.bounce_window_ms =
                    next.clamp(BOUNCE_WINDOW_MS_MIN, BOUNCE_WINDOW_MS_MAX);
                self.hold_release_test
                    .set_bounce_window_ms(self.config.hold_release.bounce_window_ms);
            }
            3 => {
                // Refresh rate
                if increase {
                    self.config.ui.refresh_rate_hz = self
                        .config
                        .ui
                        .refresh_rate_hz
                        .saturating_add(10)
                        .min(REFRESH_RATE_HZ_MAX);
                } else {
                    self.config.ui.refresh_rate_hz = self
                        .config
                        .ui
                        .refresh_rate_hz
                        .saturating_sub(10)
                        .max(REFRESH_RATE_HZ_MIN);
                }
            }
            4 => {
                // Theme toggle
                self.toggle_theme();
            }
            5 => {
                // FN mode cycle
                self.cycle_fn_mode();
            }
            6 => {
                // Warning duration
                if increase {
                    self.config.ui.warning_duration_secs = self
                        .config
                        .ui
                        .warning_duration_secs
                        .saturating_add(1)
                        .min(WARNING_DURATION_SECS_MAX);
                } else {
                    self.config.ui.warning_duration_secs = self
                        .config
                        .ui
                        .warning_duration_secs
                        .saturating_sub(1)
                        .max(WARNING_DURATION_SECS_MIN);
                }
            }
            _ => {}
        }
    }

    /// Save current config to disk
    pub fn save_config(&mut self) {
        match self.config.save() {
            Ok(()) => self.set_status("Config saved".to_string()),
            Err(e) => self.set_status(format!("Save failed: {}", e)),
        }
    }

    /// Get shortcut overlay info if one should be displayed
    pub fn shortcut_overlay(&self) -> Option<(&str, Option<&str>)> {
        if let (Some(combo), Some(time)) = (&self.last_shortcut_combo, self.last_shortcut_time) {
            let duration = self.config.ui.warning_duration_secs as u64;
            if time.elapsed().as_secs() < duration {
                return Some((combo.as_str(), self.last_shortcut_desc.as_deref()));
            }
        }
        None
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new(Config::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keyboard::{KeyCode, KeyEvent, KeyEventType};
    use crate::tests::KeyboardTest;
    use std::time::Instant;

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

    #[test]
    fn app_new_default_state() {
        let app = App::default();
        assert_eq!(app.view, AppView::Dashboard);
        assert_eq!(app.state, AppState::Running);
        assert_eq!(app.total_events, 0);
        assert!(app.shortcuts_enabled);
    }

    #[test]
    fn app_process_event_distributes_to_all_tests() {
        let mut app = App::default();

        // Send a key press through the whole pipeline
        app.process_event(&press(30, 1000)); // Key 'A'
        assert_eq!(app.total_events, 1);

        // Verify it reached the keyboard state tracker
        assert_eq!(app.keyboard_state.current_rollover(), 1);

        // Verify it reached the rollover test
        assert_eq!(app.rollover_test.current_count(), 1);

        // Send release
        app.process_event(&release(30, 1000));
        assert_eq!(app.total_events, 2);
        assert_eq!(app.keyboard_state.current_rollover(), 0);
    }

    #[test]
    fn app_paused_ignores_events() {
        let mut app = App::default();
        app.toggle_pause();
        assert_eq!(app.state, AppState::Paused);

        app.process_event(&press(30, 1000));
        assert_eq!(app.total_events, 0); // Event ignored
    }

    #[test]
    fn app_resume_accepts_events() {
        let mut app = App::default();
        app.toggle_pause();
        app.toggle_pause(); // Resume
        assert_eq!(app.state, AppState::Running);

        app.process_event(&press(30, 1000));
        assert_eq!(app.total_events, 1);
    }

    #[test]
    fn app_reset_all_clears_state() {
        let mut app = App::default();
        app.process_event(&press(30, 1000));
        app.process_event(&press(31, 1000));
        app.process_event(&release(30, 1000));
        app.process_event(&release(31, 1000));
        assert_eq!(app.total_events, 4);

        app.reset_all();
        assert_eq!(app.total_events, 0);
        assert_eq!(app.keyboard_state.current_rollover(), 0);
        assert_eq!(app.keyboard_state.max_rollover(), 0);
    }

    #[test]
    fn app_view_navigation() {
        let mut app = App::default();
        assert_eq!(app.view, AppView::Dashboard);

        app.next_view();
        assert_eq!(app.view, AppView::PollingRate);

        app.next_view();
        assert_eq!(app.view, AppView::HoldRelease);

        app.prev_view();
        assert_eq!(app.view, AppView::PollingRate);
    }

    #[test]
    fn app_view_navigation_wraps() {
        let mut app = App::default();
        app.prev_view(); // Wrap from Dashboard to Help
        assert_eq!(app.view, AppView::Help);

        app.next_view(); // Wrap from Help to Dashboard
        assert_eq!(app.view, AppView::Dashboard);
    }

    #[test]
    fn app_quit() {
        let mut app = App::default();
        app.quit();
        assert_eq!(app.state, AppState::Quitting);
    }

    #[test]
    fn app_multi_key_rollover_tracking() {
        let mut app = App::default();

        // Press 4 keys simultaneously
        app.process_event(&press(30, 1000)); // A
        app.process_event(&press(31, 1000)); // S
        app.process_event(&press(32, 1000)); // D
        app.process_event(&press(33, 1000)); // F

        assert_eq!(app.keyboard_state.max_rollover(), 4);
        assert_eq!(app.rollover_test.max_rollover(), 4);

        // Release all
        app.process_event(&release(30, 1000));
        app.process_event(&release(31, 1000));
        app.process_event(&release(32, 1000));
        app.process_event(&release(33, 1000));

        assert_eq!(app.keyboard_state.current_rollover(), 0);
        assert_eq!(app.keyboard_state.max_rollover(), 4); // Max preserved
    }

    #[test]
    fn app_event_timing_receives_events() {
        let mut app = App::default();

        app.process_event(&press(30, 5000)); // 5ms interval
        app.process_event(&press(31, 3000)); // 3ms interval

        let results = app.event_timing_test.get_results();
        // Should have results (at least headers + samples)
        assert!(!results.is_empty());
    }

    #[test]
    fn app_generate_report_includes_all_tests() {
        let mut app = App::default();
        app.process_event(&press(30, 1000));
        app.process_event(&release(30, 1000));

        let report = app.generate_report();
        assert_eq!(report.summary.total_events, 2);

        // Report should have entries for all 8 tests
        // (they may be empty if no relevant events occurred for that test type)
        let json = report.to_json().expect("Failed to serialize");
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
    fn app_reset_current_only_resets_active_view() {
        let mut app = App::default();

        // Process events
        app.process_event(&press(30, 1000));
        app.process_event(&release(30, 1000));
        assert_eq!(app.total_events, 2);

        // Switch to Rollover view and reset just that test
        app.view = AppView::Rollover;
        app.reset_current();

        // Rollover test should be reset
        assert_eq!(app.rollover_test.max_rollover(), 0);

        // But other tests should retain their state
        assert_eq!(app.total_events, 2); // Total events unchanged
    }

    #[test]
    fn app_toggle_shortcuts() {
        let mut app = App::default();
        assert!(app.shortcuts_enabled);

        app.toggle_shortcuts();
        assert!(!app.shortcuts_enabled);

        app.toggle_shortcuts();
        assert!(app.shortcuts_enabled);
    }

    #[test]
    fn app_status_message() {
        let mut app = App::default();
        assert!(app.status_message.is_none());

        app.set_status("Test message".to_string());
        assert_eq!(app.status_message.as_deref(), Some("Test message"));
        assert!(app.status_time.is_some());
    }

    #[test]
    fn app_shortcut_detection_via_pipeline() {
        let mut app = App::default();
        // A bare Ctrl press only arrives through evdev (or the kitty protocol)
        app.set_input_source(InputSource::Evdev);

        // Press Ctrl (scancode 29 = Left Ctrl)
        app.process_event(&press(29, 1000));
        // Press C (scancode 46 = C)
        app.process_event(&press(46, 1000));

        // Shortcut test should have detected Ctrl+C
        let results = app.shortcut_test.get_results();
        let labels: Vec<&str> = results.iter().map(|r| r.label.as_str()).collect();
        assert!(labels.contains(&"Total Shortcuts"));
    }

    #[test]
    fn app_toggle_theme() {
        let mut app = App::default();
        assert_eq!(app.config.ui.theme, Theme::Dark);

        app.toggle_theme();
        assert_eq!(app.config.ui.theme, Theme::Light);

        app.toggle_theme();
        assert_eq!(app.config.ui.theme, Theme::Dark);
    }

    #[test]
    fn app_settings_items() {
        let app = App::default();
        let items = app.settings_items();
        assert!(items.len() >= 6);
        assert_eq!(items[0].label, "Polling Test Duration (sec)");
        assert_eq!(items[4].label, "Theme");
    }

    #[test]
    fn app_adjust_setting() {
        let mut app = App::default();
        let original = app.config.polling.test_duration_secs;

        app.settings_selected = 0;
        app.adjust_setting(true);
        assert_eq!(app.config.polling.test_duration_secs, original + 5);

        app.adjust_setting(false);
        assert_eq!(app.config.polling.test_duration_secs, original);
    }

    #[test]
    fn app_settings_view() {
        let app = App {
            view: AppView::Settings,
            ..App::default()
        };
        let results = app.current_results();
        assert!(results.is_empty()); // Settings returns empty results (uses settings_items instead)
    }

    #[test]
    fn app_shortcut_overlay_timeout() {
        let app = App::default();
        // No shortcut detected yet
        assert!(app.shortcut_overlay().is_none());
    }

    /// An instant `ago` in the past (falls back to now on a freshly booted clock).
    fn ago(ago: Duration) -> Instant {
        Instant::now().checked_sub(ago).unwrap_or_else(Instant::now)
    }

    fn event_at(key: u16, kind: KeyEventType, at: Instant) -> KeyEvent {
        KeyEvent::new(KeyCode(key), kind, at, 1000)
    }

    /// Run the auto test (with real releases) up to `step`.
    fn app_at_auto_step(step: StepKind) -> App {
        let mut app = App::default();
        app.set_input_source(InputSource::Evdev);
        app.start_auto_test();
        app.tick();
        while app.auto_test.current_step() != Some(step) {
            app.skip_auto_step();
            app.tick();
        }
        app
    }

    #[test]
    fn stuck_threshold_relaxed_for_hold_and_rollover_steps() {
        let configured = Duration::from_millis(Config::default().stickiness.stuck_threshold_ms);
        let relaxed = Duration::from_millis(HOLD_STEP_STUCK_THRESHOLD_MS);

        let mut app = app_at_auto_step(StepKind::Sweep);
        assert_eq!(app.stickiness_test.threshold(), configured);

        app.skip_auto_step();
        app.tick();
        assert_eq!(app.auto_test.current_step(), Some(StepKind::Hold));
        assert_eq!(app.stickiness_test.threshold(), relaxed);

        // Hold -> Rollover must keep the relaxed threshold: the user is asked
        // to hold as many keys as possible, which takes a few seconds.
        app.skip_auto_step();
        app.tick();
        assert_eq!(app.auto_test.current_step(), Some(StepKind::Rollover));
        assert_eq!(app.stickiness_test.threshold(), relaxed);

        app.skip_auto_step();
        app.tick();
        assert_eq!(app.auto_test.current_step(), Some(StepKind::RapidTap));
        assert_eq!(app.stickiness_test.threshold(), configured);
    }

    #[test]
    fn stuck_threshold_restored_when_aborted_during_rollover() {
        let mut app = app_at_auto_step(StepKind::Rollover);
        app.abort_auto_test();
        assert_eq!(
            app.stickiness_test.threshold(),
            Duration::from_millis(app.config.stickiness.stuck_threshold_ms)
        );
    }

    #[test]
    fn stuck_threshold_for_step_policy() {
        assert_eq!(stuck_threshold_for_step(None, 2000), 2000);
        assert_eq!(stuck_threshold_for_step(Some(StepKind::Sweep), 2000), 2000);
        assert_eq!(
            stuck_threshold_for_step(Some(StepKind::Hold), 2000),
            HOLD_STEP_STUCK_THRESHOLD_MS
        );
        assert_eq!(
            stuck_threshold_for_step(Some(StepKind::Rollover), 2000),
            HOLD_STEP_STUCK_THRESHOLD_MS
        );
        // A user threshold above the relaxed one is never lowered
        assert_eq!(
            stuck_threshold_for_step(Some(StepKind::Rollover), 30_000),
            30_000
        );
    }

    #[test]
    fn rollover_step_long_hold_is_not_flagged() {
        // The user takes 3 s to get 6 keys down, then lets go.
        let mut app = app_at_auto_step(StepKind::Rollover);
        let t0 = ago(Duration::from_secs(3));
        for key in 30..36 {
            app.process_event(&event_at(key, KeyEventType::Press, t0));
        }
        app.tick();
        assert!(app.stickiness_test.flagged_keys().is_empty());
        assert!(app.stickiness_test.stuck_keys().is_empty());
        for key in 30..36 {
            app.process_event(&event_at(key, KeyEventType::Release, Instant::now()));
        }
        app.tick();
        assert!(app.stickiness_test.stuck_keys().is_empty());
    }

    #[test]
    fn key_still_down_after_rollover_step_is_stuck() {
        // A key that never comes back up is still caught once the step is over.
        let mut app = app_at_auto_step(StepKind::Rollover);
        app.process_event(&event_at(
            30,
            KeyEventType::Press,
            ago(Duration::from_secs(3)),
        ));
        app.skip_auto_step();
        app.tick();
        assert_eq!(app.auto_test.current_step(), Some(StepKind::RapidTap));
        let stuck: Vec<KeyCode> = app
            .stickiness_test
            .stuck_keys()
            .iter()
            .map(|(k, _)| *k)
            .collect();
        assert_eq!(stuck, vec![KeyCode(30)]);
    }

    #[test]
    fn release_during_pause_is_not_lost() {
        let mut app = App::default();
        app.set_input_source(InputSource::Evdev);
        // Down for 1 s (below the 2 s threshold) when Space pauses the app
        app.process_event(&event_at(
            30,
            KeyEventType::Press,
            ago(Duration::from_secs(1)),
        ));
        app.toggle_pause();
        // Key released while paused (e.g. the user lets go after pausing)
        app.process_event(&event_at(30, KeyEventType::Release, Instant::now()));
        app.toggle_pause();
        // Time passing after the resume, simulated with a shorter threshold:
        // a key the tests still thought was down would now be flagged.
        app.stickiness_test.set_threshold(250);
        app.tick();

        assert_eq!(app.keyboard_state.current_rollover(), 0);
        assert_eq!(app.rollover_test.current_count(), 0);
        assert!(app.stickiness_test.held_keys().is_empty());
        assert!(app.stickiness_test.flagged_keys().is_empty());
        assert!(app.stickiness_test.stuck_keys().is_empty());
    }

    #[test]
    fn pause_still_ignores_new_keystrokes() {
        let mut app = App::default();
        app.set_input_source(InputSource::Evdev);
        app.toggle_pause();
        app.process_event(&press(30, 1000));
        app.process_event(&release(30, 1000));
        // A release for a key the tests never saw go down is not recorded
        app.process_event(&release(31, 1000));
        assert_eq!(app.total_events, 0);
        assert_eq!(app.keyboard_state.total_events(), 0);
    }

    #[test]
    fn key_held_through_pause_is_still_detected_as_stuck() {
        let mut app = App::default();
        app.set_input_source(InputSource::Evdev);
        app.process_event(&event_at(
            30,
            KeyEventType::Press,
            ago(Duration::from_secs(3)),
        ));
        app.toggle_pause();
        app.toggle_pause();
        app.tick();
        assert_eq!(app.stickiness_test.stuck_keys().len(), 1);
    }

    #[test]
    fn stuck_threshold_setting_is_bounded() {
        // Settings view uses the same bounds a loaded config is clamped to
        let mut app = App {
            settings_selected: 1,
            ..App::default()
        };
        app.config.stickiness.stuck_threshold_ms = STUCK_THRESHOLD_MS_MAX;
        app.adjust_setting(true);
        assert_eq!(
            app.config.stickiness.stuck_threshold_ms,
            STUCK_THRESHOLD_MS_MAX
        );
        assert_eq!(
            app.stickiness_test.threshold(),
            Duration::from_millis(STUCK_THRESHOLD_MS_MAX)
        );

        app.config.stickiness.stuck_threshold_ms = STUCK_THRESHOLD_MS_MIN;
        app.adjust_setting(false);
        assert_eq!(
            app.config.stickiness.stuck_threshold_ms,
            STUCK_THRESHOLD_MS_MIN
        );
    }

    #[test]
    fn one_unplug_of_a_multi_node_keyboard_is_one_disconnect() {
        let mut app = App::default();
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        // Keyboard, consumer-control and NKRO nodes vanish together
        app.keyboard_disconnected_at("Gaming Keyboard", ms(0));
        app.keyboard_disconnected_at("Gaming Keyboard Consumer Control", ms(3));
        app.keyboard_disconnected_at("Gaming Keyboard", ms(20));
        assert_eq!(app.keyboard_disconnects(), 1);
        // It comes back and drops out again: a second disconnect
        app.keyboard_connected();
        app.keyboard_disconnected_at("Gaming Keyboard", ms(900));
        assert_eq!(app.keyboard_disconnects(), 2);
        // A later, separate drop without a reconnect seen in between
        app.keyboard_disconnected_at("Gaming Keyboard", ms(10_000));
        assert_eq!(app.keyboard_disconnects(), 3);
        // Each device is named once
        let connection = app
            .findings
            .iter()
            .find(|f| f.category == "Connection")
            .unwrap();
        assert!(connection.title.contains("3 time(s)"));
        assert!(connection
            .detail
            .starts_with("Gaming Keyboard, Gaming Keyboard Consumer Control. "));
    }

    #[test]
    fn keys_typed_as_the_auto_test_ends_stay_test_input() {
        let mut app = app_at_auto_step(StepKind::RapidTap);
        assert!(!app.auto_test_just_ended());
        app.abort_auto_test();
        let ended = app.auto_test.finished_at().unwrap();
        let ms = |n: u64| ended + Duration::from_millis(n);
        assert!(app.auto_test_just_ended_at(ms(100)));
        // The user keeps tapping for a while after the step ends
        for t in [80, 240, 400, 560] {
            app.process_event(&event_at(30, KeyEventType::Press, ms(t)));
            app.process_event(&event_at(30, KeyEventType::Release, ms(t + 40)));
        }
        assert!(app.auto_test_just_ended_at(ms(560) + AFTER_RUN_GRACE - Duration::from_millis(1)));
        // Once the keyboard has been quiet for the grace period, commands work
        assert!(!app.auto_test_just_ended_at(ms(560) + AFTER_RUN_GRACE));

        // A run that ends while nobody is typing frees the keys after the
        // grace period, and a fresh app has no run to protect
        let mut app = app_at_auto_step(StepKind::Sweep);
        app.abort_auto_test();
        let ended = app.auto_test.finished_at().unwrap();
        assert!(!app.auto_test_just_ended_at(ended + AFTER_RUN_GRACE));
        assert!(!App::default().auto_test_just_ended());
    }

    #[test]
    fn resets_keep_a_jammed_key_down() {
        // Backspace jammed 5 s ago; "potentially stuck" makes the user reset
        // to re-check. The key sends no new press, so it must not vanish.
        let jammed = || {
            let mut app = App::default();
            app.set_input_source(InputSource::Evdev);
            app.process_event(&event_at(
                14,
                KeyEventType::Press,
                ago(Duration::from_secs(5)),
            ));
            app.process_event(&press(30, 1000));
            app.process_event(&release(30, 1000));
            app.tick();
            app
        };
        let stuck = |app: &App| -> Vec<KeyCode> {
            app.stickiness_test
                .stuck_keys()
                .iter()
                .map(|(k, _)| *k)
                .collect()
        };

        let mut app = jammed();
        app.reset_all();
        assert_eq!(stuck(&app), vec![KeyCode(14)]);
        assert_eq!(app.keyboard_state.pressed_keys(), &[KeyCode(14)]);
        assert_eq!(app.rollover_test.current_count(), 1);
        app.refresh_findings();
        assert!(app.findings.iter().any(|f| f.category == "Stuck key"));
        // ...and an auto test started after it still knows the key is down
        app.start_auto_test();
        assert_eq!(stuck(&app), vec![KeyCode(14)]);

        // Ctrl+R in the Sticky view
        let mut app = jammed();
        app.view = AppView::Stickiness;
        app.reset_current();
        assert_eq!(stuck(&app), vec![KeyCode(14)]);
        // Released keys are forgotten as before
        assert_eq!(app.stickiness_test.held_keys().len(), 1);

        // Ctrl+R in the NKRO view keeps the current count right
        let mut app = jammed();
        app.view = AppView::Rollover;
        app.reset_current();
        assert_eq!(app.rollover_test.current_count(), 1);
        assert_eq!(app.rollover_test.max_rollover(), 1);
    }

    #[test]
    fn polling_duration_and_bounce_window_settings_are_bounded() {
        // Holding an arrow key in the Settings view must not walk a value
        // out of the range a loaded config is clamped to
        let mut app = App {
            settings_selected: 0,
            ..App::default()
        };
        app.config.polling.test_duration_secs = POLLING_DURATION_SECS_MAX;
        app.adjust_setting(true);
        assert_eq!(
            app.config.polling.test_duration_secs,
            POLLING_DURATION_SECS_MAX
        );
        assert_eq!(app.polling_test.duration_secs(), POLLING_DURATION_SECS_MAX);
        app.config.polling.test_duration_secs = POLLING_DURATION_SECS_MIN;
        app.adjust_setting(false);
        assert_eq!(
            app.config.polling.test_duration_secs,
            POLLING_DURATION_SECS_MIN
        );

        app.settings_selected = 2;
        for _ in 0..100 {
            app.adjust_setting(true);
        }
        assert_eq!(
            app.config.hold_release.bounce_window_ms,
            BOUNCE_WINDOW_MS_MAX
        );
        assert_eq!(
            app.hold_release_test.bounce_window_ms(),
            BOUNCE_WINDOW_MS_MAX
        );
        for _ in 0..100 {
            app.adjust_setting(false);
        }
        assert_eq!(
            app.config.hold_release.bounce_window_ms,
            BOUNCE_WINDOW_MS_MIN
        );
        assert_eq!(
            app.hold_release_test.bounce_window_ms(),
            BOUNCE_WINDOW_MS_MIN
        );
        assert_eq!(
            app.settings_items()[2].value,
            BOUNCE_WINDOW_MS_MIN.to_string()
        );
    }
}
