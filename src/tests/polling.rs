//! Polling rate test module

use super::{KeyboardTest, TestResult, ResultStatus};
use crate::keyboard::{KeyEvent, KeyEventType};
use crate::utils::MinMaxExt;
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
                self.min_interval_us.update_min(interval_us);
                self.max_interval_us.update_max(interval_us);
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

        // Tooltip: Explain what this test measures
        results.push(TestResult::info(
            "--- What This Measures ---",
            "",
        ));
        results.push(TestResult::info(
            "Polling rate = how often",
            "keyboard sends data to PC",
        ));
        results.push(TestResult::info(
            "Higher Hz = lower latency",
            "1000Hz=1ms, 125Hz=8ms delay",
        ));
        results.push(TestResult::info(
            "Look for: 1000Hz gaming,",
            "125Hz standard, low jitter",
        ));
        results.push(TestResult::info("", ""));

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::test_helpers::{press_at, release_at, DEFAULT_KEY};

    #[test]
    fn new_test_has_no_data() {
        let test = PollingRateTest::new(10);
        assert!(test.avg_rate_hz().is_none());
        assert!(test.min_rate_hz().is_none());
        assert!(test.max_rate_hz().is_none());
        assert!(test.jitter_us().is_none());
        assert_eq!(test.progress(), 0.0);
    }

    #[test]
    fn avg_rate_hz_1000hz() {
        let mut test = PollingRateTest::new(10);
        // Manually set intervals of 1000us each = 1000 Hz
        test.intervals_us = vec![1000, 1000, 1000, 1000, 1000];

        let rate = test.avg_rate_hz().unwrap();
        assert!((rate - 1000.0).abs() < 0.01);
    }

    #[test]
    fn avg_rate_hz_125hz() {
        let mut test = PollingRateTest::new(10);
        // 8000us intervals = 125 Hz
        test.intervals_us = vec![8000, 8000, 8000, 8000];

        let rate = test.avg_rate_hz().unwrap();
        assert!((rate - 125.0).abs() < 0.01);
    }

    #[test]
    fn min_max_rate_calculation() {
        let mut test = PollingRateTest::new(10);
        test.min_interval_us = Some(500);  // 2000 Hz
        test.max_interval_us = Some(2000); // 500 Hz

        let max_rate = test.max_rate_hz().unwrap();
        assert!((max_rate - 2000.0).abs() < 0.01);

        let min_rate = test.min_rate_hz().unwrap();
        assert!((min_rate - 500.0).abs() < 0.01);
    }

    #[test]
    fn jitter_calculation_uniform() {
        let mut test = PollingRateTest::new(10);
        // All same intervals = 0 jitter
        test.intervals_us = vec![1000, 1000, 1000, 1000, 1000];

        let jitter = test.jitter_us().unwrap();
        assert!(jitter.abs() < 0.01);
    }

    #[test]
    fn jitter_calculation_varied() {
        let mut test = PollingRateTest::new(10);
        // Varied intervals should have non-zero jitter
        test.intervals_us = vec![500, 1500, 500, 1500, 500, 1500];

        let jitter = test.jitter_us().unwrap();
        assert!(jitter > 0.0);
        // Standard deviation of [500, 1500, ...] with mean 1000 is 500
        assert!((jitter - 500.0).abs() < 1.0);
    }

    #[test]
    fn jitter_requires_two_samples() {
        let mut test = PollingRateTest::new(10);
        test.intervals_us = vec![1000];
        assert!(test.jitter_us().is_none());
    }

    #[test]
    fn process_event_ignores_release() {
        let mut test = PollingRateTest::new(10);
        let now = Instant::now();

        test.process_event(&release_at(DEFAULT_KEY, now));

        assert_eq!(test.event_count, 0);
        assert!(test.start_time.is_none());
    }

    #[test]
    fn process_event_starts_test() {
        let mut test = PollingRateTest::new(10);
        let now = Instant::now();

        test.process_event(&press_at(DEFAULT_KEY, now, 0));

        assert!(test.start_time.is_some());
        assert_eq!(test.event_count, 1);
    }

    #[test]
    fn process_event_filters_large_intervals() {
        let mut test = PollingRateTest::new(10);
        let now = Instant::now();

        // First event
        test.process_event(&press_at(DEFAULT_KEY, now, 0));

        // Simulate a long pause (200ms = 200000us) - this should be filtered
        let later = now + Duration::from_millis(200);
        test.process_event(&press_at(DEFAULT_KEY, later, 0));

        // The large interval should not be recorded
        assert!(test.intervals_us.is_empty());
    }

    #[test]
    fn reset_clears_all() {
        let mut test = PollingRateTest::new(10);
        test.start_time = Some(Instant::now());
        test.intervals_us = vec![1000, 2000];
        test.event_count = 5;
        test.min_interval_us = Some(500);
        test.max_interval_us = Some(2000);

        test.reset();

        assert!(test.start_time.is_none());
        assert!(test.intervals_us.is_empty());
        assert_eq!(test.event_count, 0);
        assert!(test.min_interval_us.is_none());
        assert!(test.max_interval_us.is_none());
    }

    #[test]
    fn is_complete_before_start() {
        let test = PollingRateTest::new(10);
        assert!(!test.is_complete());
    }

    #[test]
    fn test_name_and_description() {
        let test = PollingRateTest::new(10);
        assert_eq!(test.name(), "Polling Rate Test");
        assert!(!test.description().is_empty());
    }
}
