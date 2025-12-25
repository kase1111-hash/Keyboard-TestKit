//! Test modules for keyboard diagnostics

mod polling;
mod stickiness;
mod rollover;
mod latency;
mod bounce;
mod shortcuts;

pub use polling::PollingRateTest;
pub use stickiness::StickinessTest;
pub use rollover::RolloverTest;
pub use latency::LatencyTest;
pub use bounce::HoldReleaseTest;
pub use shortcuts::ShortcutTest;

use crate::keyboard::KeyEvent;
use std::time::Instant;

/// Common trait for all keyboard tests
pub trait KeyboardTest {
    /// Name of the test
    fn name(&self) -> &'static str;

    /// Short description
    fn description(&self) -> &'static str;

    /// Process a key event
    fn process_event(&mut self, event: &KeyEvent);

    /// Check if test is complete
    fn is_complete(&self) -> bool;

    /// Get test results as formatted strings
    fn get_results(&self) -> Vec<TestResult>;

    /// Reset the test
    fn reset(&mut self);
}

/// A single test result entry
#[derive(Debug, Clone)]
pub struct TestResult {
    pub label: String,
    pub value: String,
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

/// Status of a test result
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultStatus {
    Ok,
    Warning,
    Error,
    Info,
}

/// Test session tracking
pub struct TestSession {
    pub start_time: Instant,
    pub tests_run: u32,
    pub events_processed: u64,
}

impl TestSession {
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
            tests_run: 0,
            events_processed: 0,
        }
    }

    pub fn elapsed_secs(&self) -> f64 {
        self.start_time.elapsed().as_secs_f64()
    }
}

impl Default for TestSession {
    fn default() -> Self {
        Self::new()
    }
}
