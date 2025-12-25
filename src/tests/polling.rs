//! Polling rate test module

use super::{KeyboardTest, TestResult, ResultStatus};
use crate::keyboard::{KeyEvent, KeyEventType};
use std::time::{Duration, Instant};

/// Test for measuring keyboard polling rate
pub struct PollingRateTest {
    /// Test duration
    duration: Duration,
    /// Start time of the test
    start_time: Option<Instant>,
    /// Recorded intervals between events (in microseconds)
    intervals_us: Vec<u64>,
    /// Last event timestamp
    last_event: Option<Instant>,
    /// Number of events recorded
    event_count: u64,
    /// Minimum interval seen
    min_interval_us: Option<u64>,
    /// Maximum interval seen
    max_interval_us: Option<u64>,
}

impl PollingRateTest {
    pub fn new(duration_secs: u64) -> Self {
        Self {
            duration: Duration::from_secs(duration_secs),
            start_time: None,
            intervals_us: Vec::with_capacity(10000),
            last_event: None,
            event_count: 0,
            min_interval_us: None,
            max_interval_us: None,
        }
    }

    /// Calculate average polling rate in Hz
    pub fn avg_rate_hz(&self) -> Option<f64> {
        if self.intervals_us.is_empty() {
            return None;
        }
        let avg_us: f64 = self.intervals_us.iter().sum::<u64>() as f64
            / self.intervals_us.len() as f64;
        if avg_us > 0.0 {
            Some(1_000_000.0 / avg_us)
        } else {
            None
        }
    }

    /// Calculate minimum polling rate
    pub fn min_rate_hz(&self) -> Option<f64> {
        self.max_interval_us.map(|us| 1_000_000.0 / us as f64)
    }

    /// Calculate maximum polling rate
    pub fn max_rate_hz(&self) -> Option<f64> {
        self.min_interval_us.map(|us| 1_000_000.0 / us as f64)
    }

    /// Get jitter (standard deviation of intervals)
    pub fn jitter_us(&self) -> Option<f64> {
        if self.intervals_us.len() < 2 {
            return None;
        }
        let mean = self.intervals_us.iter().sum::<u64>() as f64 / self.intervals_us.len() as f64;
        let variance = self.intervals_us.iter()
            .map(|&x| {
                let diff = x as f64 - mean;
                diff * diff
            })
            .sum::<f64>() / self.intervals_us.len() as f64;
        Some(variance.sqrt())
    }

    /// Get test progress (0.0 to 1.0)
    pub fn progress(&self) -> f64 {
        match self.start_time {
            Some(start) => {
                let elapsed = start.elapsed();
                (elapsed.as_secs_f64() / self.duration.as_secs_f64()).min(1.0)
            }
            None => 0.0,
        }
    }
}

impl KeyboardTest for PollingRateTest {
    fn name(&self) -> &'static str {
        "Polling Rate Test"
    }

    fn description(&self) -> &'static str {
        "Measures keyboard polling rate by analyzing event timing"
    }

    fn process_event(&mut self, event: &KeyEvent) {
        // Only count key press events for cleaner measurement
        if event.event_type != KeyEventType::Press {
            return;
        }

        // Start the test on first event
        if self.start_time.is_none() {
            self.start_time = Some(Instant::now());
        }

        // Record interval from last event
        if let Some(last) = self.last_event {
            let interval_us = event.timestamp.duration_since(last).as_micros() as u64;

            // Filter out unreasonably large intervals (likely pauses in typing)
            if interval_us < 100_000 { // Less than 100ms
                self.intervals_us.push(interval_us);

                self.min_interval_us = Some(
                    self.min_interval_us.map(|m| m.min(interval_us)).unwrap_or(interval_us)
                );
                self.max_interval_us = Some(
                    self.max_interval_us.map(|m| m.max(interval_us)).unwrap_or(interval_us)
                );
            }
        }

        self.last_event = Some(event.timestamp);
        self.event_count += 1;
    }

    fn is_complete(&self) -> bool {
        match self.start_time {
            Some(start) => start.elapsed() >= self.duration,
            None => false,
        }
    }

    fn get_results(&self) -> Vec<TestResult> {
        let mut results = Vec::new();

        results.push(TestResult::info(
            "Events Recorded",
            format!("{}", self.event_count),
        ));

        if let Some(avg) = self.avg_rate_hz() {
            let status = if avg >= 900.0 {
                ResultStatus::Ok
            } else if avg >= 450.0 {
                ResultStatus::Warning
            } else {
                ResultStatus::Error
            };
            results.push(TestResult::new(
                "Average Rate",
                format!("{:.1} Hz", avg),
                status,
            ));
        }

        if let Some(min) = self.min_rate_hz() {
            results.push(TestResult::info("Min Rate", format!("{:.1} Hz", min)));
        }

        if let Some(max) = self.max_rate_hz() {
            results.push(TestResult::info("Max Rate", format!("{:.1} Hz", max)));
        }

        if let Some(jitter) = self.jitter_us() {
            let status = if jitter < 500.0 {
                ResultStatus::Ok
            } else if jitter < 2000.0 {
                ResultStatus::Warning
            } else {
                ResultStatus::Error
            };
            results.push(TestResult::new(
                "Jitter",
                format!("{:.1} μs", jitter),
                status,
            ));
        }

        results.push(TestResult::info(
            "Progress",
            format!("{:.0}%", self.progress() * 100.0),
        ));

        results
    }

    fn reset(&mut self) {
        self.start_time = None;
        self.intervals_us.clear();
        self.last_event = None;
        self.event_count = 0;
        self.min_interval_us = None;
        self.max_interval_us = None;
    }
}
