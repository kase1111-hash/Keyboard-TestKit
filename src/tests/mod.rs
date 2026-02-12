//! Test modules for keyboard diagnostics
//!
//! This module provides various keyboard tests that implement the [`KeyboardTest`] trait.
//!
//! ## Available Tests
//!
//! | Test | Description |
//! |------|-------------|
//! | [`PollingRateTest`] | Measures keyboard polling frequency (Hz) and jitter |
//! | [`StickinessTest`] | Detects stuck keys that fail to release properly |
//! | [`RolloverTest`] | Tests N-Key Rollover (NKRO) and ghosting detection |
//! | [`EventTimingTest`] | Measures inter-event timing intervals per-key and globally |
//! | [`HoldReleaseTest`] | Analyzes key hold duration and mechanical bounce |
//! | [`ShortcutTest`] | Validates keyboard shortcut combinations |
//! | [`VirtualKeyboardTest`] | Compares physical vs virtual key events |
//! | [`OemKeyTest`] | Captures OEM keys and provides FN key restoration |
//!
//! ## Usage
//!
//! All tests implement the [`KeyboardTest`] trait:
//!
//! ```no_run
//! use keyboard_testkit::tests::{KeyboardTest, PollingRateTest};
//! use keyboard_testkit::keyboard::{KeyEvent, KeyEventType, KeyCode};
//! use std::time::Instant;
//!
//! let mut test = PollingRateTest::new(10); // 10 second test duration
//!
//! // Process events as they arrive
//! // test.process_event(&event);
//!
//! // Get results for display
//! let results = test.get_results();
//! for result in results {
//!     println!("{}: {}", result.label, result.value);
//! }
//!
//! // Reset for a new test session
//! test.reset();
//! ```

mod bounce;
mod latency;
mod oem_keys;
mod polling;
mod rollover;
mod shortcuts;
mod stickiness;
mod virtual_detect;

#[cfg(test)]
pub mod test_helpers;

pub use bounce::HoldReleaseTest;
pub use latency::EventTimingTest;
pub use oem_keys::OemKeyTest;
pub use polling::PollingRateTest;
pub use rollover::RolloverTest;
pub use shortcuts::ShortcutTest;
pub use stickiness::StickinessTest;
pub use virtual_detect::VirtualKeyboardTest;

use crate::keyboard::KeyEvent;
use std::time::Instant;

/// Common trait for all keyboard tests.
///
/// Each test processes keyboard events and produces results that can be
/// displayed in the UI. Tests run continuously until explicitly reset.
pub trait KeyboardTest {
    /// Returns the display name of the test.
    fn name(&self) -> &'static str;

    /// Returns a brief description of what the test measures.
    fn description(&self) -> &'static str;

    /// Processes a single keyboard event.
    ///
    /// Called for every key press and release. The test should update its
    /// internal state and statistics based on the event data.
    fn process_event(&mut self, event: &KeyEvent);

    /// Returns `true` if the test has completed.
    ///
    /// Most tests run continuously and always return `false`.
    fn is_complete(&self) -> bool;

    /// Returns the current test results as a list of labeled values.
    ///
    /// Results include metrics, statistics, and status indicators.
    fn get_results(&self) -> Vec<TestResult>;

    /// Resets all test state to initial values.
    fn reset(&mut self);
}

/// A single test result entry for display in the UI.
///
/// Each result has a label, value, and status that determines its visual styling.
#[derive(Debug, Clone)]
pub struct TestResult {
    /// The metric or statistic name (e.g., "Polling Rate", "Max Rollover")
    pub label: String,
    /// The measured value (e.g., "1000 Hz", "6KRO")
    pub value: String,
    /// Status indicator that affects visual styling in the UI
    pub status: ResultStatus,
}

impl TestResult {
    pub fn new(label: impl Into<String>, value: impl Into<String>, status: ResultStatus) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            status,
        }
    }

    pub fn ok(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self::new(label, value, ResultStatus::Ok)
    }

    pub fn warning(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self::new(label, value, ResultStatus::Warning)
    }

    pub fn error(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self::new(label, value, ResultStatus::Error)
    }

    pub fn info(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self::new(label, value, ResultStatus::Info)
    }
}

/// Status indicator for a test result.
///
/// Determines the visual styling (color) of the result in the UI:
/// - `Ok` - Green, indicates good/passing metrics
/// - `Warning` - Yellow, indicates marginal or concerning values
/// - `Error` - Red, indicates failing or problematic values
/// - `Info` - Default/gray, neutral informational display
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultStatus {
    /// Good/passing result (displayed in green)
    Ok,
    /// Marginal or concerning result (displayed in yellow)
    Warning,
    /// Failing or problematic result (displayed in red)
    Error,
    /// Neutral informational result (displayed in default color)
    Info,
}

/// Tracks overall test session statistics.
///
/// Used to aggregate metrics across all tests during a single testing session.
pub struct TestSession {
    /// When the session started
    pub start_time: Instant,
    /// Number of distinct tests that have been run
    pub tests_run: u32,
    /// Total keyboard events processed across all tests
    pub events_processed: u64,
}

impl TestSession {
    /// Creates a new test session starting now.
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
            tests_run: 0,
            events_processed: 0,
        }
    }

    /// Returns the elapsed time since the session started, in seconds.
    pub fn elapsed_secs(&self) -> f64 {
        self.start_time.elapsed().as_secs_f64()
    }
}

impl Default for TestSession {
    fn default() -> Self {
        Self::new()
    }
}
