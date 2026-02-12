//! Main application state and logic

use crate::config::{Config, Theme};
use crate::keyboard::remap::FnKeyMode;
use crate::keyboard::{KeyEvent, KeyboardState};
use crate::report::{ReportInput, SessionReport};
use crate::tests::{
    EventTimingTest, HoldReleaseTest, KeyboardTest, OemKeyTest, PollingRateTest, RolloverTest,
    ShortcutTest, StickinessTest, TestResult, VirtualKeyboardTest,
};
use crate::ui::theme::ThemeColors;
use crate::ui::widgets::SettingsItem;
use std::path::Path;
use std::time::Instant;

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
            Self::Help => 9,
            Self::Settings => 10,
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
}

impl App {
    pub fn new(config: Config) -> Self {
        // Set up OEM test with config settings
        let mut oem_test = OemKeyTest::new();
        oem_test.set_fn_mode(config.oem_keys.fn_mode);

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

        Self {
            view: AppView::Dashboard,
            state: AppState::Running,
            config: config.clone(),
            keyboard_state: KeyboardState::new(),
            polling_test: PollingRateTest::new(config.polling.test_duration_secs),
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
        }
    }

    /// Process a keyboard event through all active tests
    pub fn process_event(&mut self, event: &KeyEvent) {
        if self.state != AppState::Running {
            return;
        }

        self.total_events += 1;
        self.keyboard_state.process_event(event);

        // Track shortcuts before processing (for overlay)
        let shortcuts_before = self.shortcut_test.recent_shortcuts(1).len();

        // Process through all tests
        self.polling_test.process_event(event);
        self.hold_release_test.process_event(event);
        self.stickiness_test.process_event(event);
        self.rollover_test.process_event(event);
        self.event_timing_test.process_event(event);
        self.shortcut_test.process_event(event);
        self.virtual_test.process_event(event);
        self.oem_test.process_event(event);

        // Check if a new shortcut was detected (for overlay)
        let shortcuts_after = self.shortcut_test.recent_shortcuts(1);
        if shortcuts_after.len() > shortcuts_before || shortcuts_before == 0 {
            if let Some(last) = shortcuts_after.first() {
                self.last_shortcut_combo = Some(last.combo.clone());
                self.last_shortcut_desc = last.description.clone();
                self.last_shortcut_time = Some(Instant::now());
            }
        }

        // Check for stuck keys periodically
        let stuck = self.stickiness_test.check_stuck_keys();
        if !stuck.is_empty() {
            self.set_status(format!("Warning: {} potentially stuck key(s)", stuck.len()));
        }
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
            self.set_status("Menu shortcuts ON (1-8)".to_string());
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

    /// Reset all tests
    pub fn reset_all(&mut self) {
        self.keyboard_state.reset();
        self.polling_test.reset();
        self.hold_release_test.reset();
        self.stickiness_test.reset();
        self.rollover_test.reset();
        self.event_timing_test.reset();
        self.shortcut_test.reset();
        self.virtual_test.reset();
        self.oem_test.reset();
        self.total_events = 0;
        self.set_status("All tests reset".to_string());
    }

    /// Reset current test
    pub fn reset_current(&mut self) {
        match self.view {
            AppView::PollingRate => {
                self.polling_test.reset();
                self.set_status("Polling rate test reset".to_string());
            }
            AppView::HoldRelease => {
                self.hold_release_test.reset();
                self.set_status("Hold/release test reset".to_string());
            }
            AppView::Stickiness => {
                self.stickiness_test.reset();
                self.set_status("Stickiness test reset".to_string());
            }
            AppView::Rollover => {
                self.rollover_test.reset();
                self.set_status("Rollover test reset".to_string());
            }
            AppView::Latency => {
                self.event_timing_test.reset();
                self.set_status("Latency test reset".to_string());
            }
            AppView::Shortcuts => {
                self.shortcut_test.reset();
                self.set_status("Shortcut test reset".to_string());
            }
            AppView::Virtual => {
                self.virtual_test.reset();
                self.set_status("Virtual detection test reset".to_string());
            }
            AppView::OemKeys => {
                self.oem_test.reset();
                self.set_status("OEM key test reset".to_string());
            }
            _ => {}
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
            AppView::PollingRate => self.polling_test.get_results(),
            AppView::HoldRelease => self.hold_release_test.get_results(),
            AppView::Stickiness => self.stickiness_test.get_results(),
            AppView::Rollover => self.rollover_test.get_results(),
            AppView::Latency => self.event_timing_test.get_results(),
            AppView::Shortcuts => self.shortcut_test.get_results(),
            AppView::Virtual => self.virtual_test.get_results(),
            AppView::OemKeys => self.oem_test.get_results(),
            AppView::Help | AppView::Settings => Vec::new(),
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

        results.push(TestResult::info(
            "Max Rollover",
            format!("{}", self.keyboard_state.max_rollover()),
        ));

        if let Some(rate) = self.keyboard_state.global_polling_rate_hz() {
            results.push(TestResult::info(
                "Est. Poll Rate",
                format!("{:.0} Hz", rate),
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
        SessionReport::new(
            ReportInput {
                start_time: self.start_time,
                total_events: self.total_events,
                polling: self.polling_test.get_results(),
                hold_release: self.hold_release_test.get_results(),
                stickiness: self.stickiness_test.get_results(),
                rollover: self.rollover_test.get_results(),
                event_timing: self.event_timing_test.get_results(),
                shortcuts: self.shortcut_test.get_results(),
                virtual_detect: self.virtual_test.get_results(),
                oem_keys: self.oem_test.get_results(),
            },
            &self.keyboard_state,
        )
    }

    /// Export session report to JSON file
    pub fn export_report(&mut self, filename: &str) -> Result<String, std::io::Error> {
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
                label: "Bounce Window (ms)".to_string(),
                value: format!("{}", self.config.hold_release.bounce_window_ms),
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
                    self.config.polling.test_duration_secs =
                        self.config.polling.test_duration_secs.saturating_add(5);
                } else {
                    self.config.polling.test_duration_secs =
                        self.config.polling.test_duration_secs.saturating_sub(5).max(5);
                }
            }
            1 => {
                // Stuck threshold
                if increase {
                    self.config.stickiness.stuck_threshold_ms =
                        self.config.stickiness.stuck_threshold_ms.saturating_add(10);
                } else {
                    self.config.stickiness.stuck_threshold_ms = self
                        .config
                        .stickiness
                        .stuck_threshold_ms
                        .saturating_sub(10)
                        .max(10);
                }
            }
            2 => {
                // Bounce window
                if increase {
                    self.config.hold_release.bounce_window_ms =
                        self.config.hold_release.bounce_window_ms.saturating_add(1);
                } else {
                    self.config.hold_release.bounce_window_ms = self
                        .config
                        .hold_release
                        .bounce_window_ms
                        .saturating_sub(1)
                        .max(1);
                }
            }
            3 => {
                // Refresh rate
                if increase {
                    self.config.ui.refresh_rate_hz =
                        self.config.ui.refresh_rate_hz.saturating_add(10).min(240);
                } else {
                    self.config.ui.refresh_rate_hz =
                        self.config.ui.refresh_rate_hz.saturating_sub(10).max(10);
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
                    self.config.ui.warning_duration_secs =
                        self.config.ui.warning_duration_secs.saturating_add(1).min(30);
                } else {
                    self.config.ui.warning_duration_secs =
                        self.config.ui.warning_duration_secs.saturating_sub(1).max(1);
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
        assert_eq!(app.total_events, 2);

        app.reset_all();
        assert_eq!(app.total_events, 0);
        assert_eq!(app.keyboard_state.current_rollover(), 0);
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
        assert_eq!(
            app.config.polling.test_duration_secs,
            original + 5
        );

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
}
