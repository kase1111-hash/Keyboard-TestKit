//! Main application state and logic

use crate::config::Config;
use crate::keyboard::{KeyboardState, KeyEvent};
use crate::tests::{
    KeyboardTest, PollingRateTest, StickinessTest, RolloverTest, LatencyTest,
    HoldReleaseTest, ShortcutTest, VirtualKeyboardTest, TestResult,
};
use crate::report::SessionReport;
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
    Help,
}

impl AppView {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Dashboard => "Dashboard",
            Self::PollingRate => "Polling",
            Self::HoldRelease => "Bounce",
            Self::Stickiness => "Sticky",
            Self::Rollover => "NKRO",
            Self::Latency => "Latency",
            Self::Shortcuts => "Shortcuts",
            Self::Virtual => "Virtual",
            Self::Help => "Help",
        }
    }

    pub fn all() -> &'static [AppView] {
        &[
            Self::Dashboard,
            Self::PollingRate,
            Self::HoldRelease,
            Self::Stickiness,
            Self::Rollover,
            Self::Latency,
            Self::Shortcuts,
            Self::Virtual,
            Self::Help,
        ]
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
            Self::Help => 8,
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
    pub latency_test: LatencyTest,
    /// Shortcut detection test
    pub shortcut_test: ShortcutTest,
    /// Virtual keyboard detection test
    pub virtual_test: VirtualKeyboardTest,
    /// Application start time
    pub start_time: Instant,
    /// Total events processed
    pub total_events: u64,
    /// Last status message
    pub status_message: Option<String>,
    /// Status message timestamp
    pub status_time: Option<Instant>,
}

impl App {
    pub fn new(config: Config) -> Self {
        Self {
            view: AppView::Dashboard,
            state: AppState::Running,
            config: config.clone(),
            keyboard_state: KeyboardState::new(),
            polling_test: PollingRateTest::new(config.polling.test_duration_secs),
            hold_release_test: HoldReleaseTest::new(config.hold_release.bounce_window_ms),
            stickiness_test: StickinessTest::new(config.stickiness.stuck_threshold_ms),
            rollover_test: RolloverTest::new(),
            latency_test: LatencyTest::new(),
            shortcut_test: ShortcutTest::new(),
            virtual_test: VirtualKeyboardTest::new(),
            start_time: Instant::now(),
            total_events: 0,
            status_message: None,
            status_time: None,
        }
    }

    /// Process a keyboard event through all active tests
    pub fn process_event(&mut self, event: &KeyEvent) {
        if self.state != AppState::Running {
            return;
        }

        self.total_events += 1;
        self.keyboard_state.process_event(event);

        // Process through all tests
        self.polling_test.process_event(event);
        self.hold_release_test.process_event(event);
        self.stickiness_test.process_event(event);
        self.rollover_test.process_event(event);
        self.latency_test.process_event(event);
        self.shortcut_test.process_event(event);
        self.virtual_test.process_event(event);

        // Check for stuck keys periodically
        let stuck = self.stickiness_test.check_stuck_keys();
        if !stuck.is_empty() {
            self.set_status(format!("Warning: {} potentially stuck key(s)", stuck.len()));
        }
    }

    /// Switch to the next view
    pub fn next_view(&mut self) {
        let current = self.view.index();
        let next = (current + 1) % AppView::all().len();
        self.view = AppView::from_index(next);
    }

    /// Switch to the previous view
    pub fn prev_view(&mut self) {
        let current = self.view.index();
        let prev = if current == 0 {
            AppView::all().len() - 1
        } else {
            current - 1
        };
        self.view = AppView::from_index(prev);
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
        self.latency_test.reset();
        self.shortcut_test.reset();
        self.virtual_test.reset();
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
                self.latency_test.reset();
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
            AppView::Latency => self.latency_test.get_results(),
            AppView::Shortcuts => self.shortcut_test.get_results(),
            AppView::Virtual => self.virtual_test.get_results(),
            AppView::Help => Vec::new(),
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
            self.start_time,
            self.total_events,
            &self.keyboard_state,
            self.polling_test.get_results(),
            self.hold_release_test.get_results(),
            self.stickiness_test.get_results(),
            self.rollover_test.get_results(),
            self.latency_test.get_results(),
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
}

impl Default for App {
    fn default() -> Self {
        Self::new(Config::default())
    }
}
