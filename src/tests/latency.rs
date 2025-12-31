//! Latency measurement test module

use super::{KeyboardTest, TestResult, ResultStatus};
use crate::keyboard::{KeyCode, KeyEvent, KeyEventType, keymap};
use std::collections::HashMap;
use std::time::Instant;

/// Per-key latency statistics
#[derive(Debug, Clone, Default)]
struct KeyLatencyStats {
    samples: Vec<u64>,
    min_us: Option<u64>,
    max_us: Option<u64>,
}

impl KeyLatencyStats {
    fn add_sample(&mut self, latency_us: u64) {
        self.samples.push(latency_us);
        self.min_us = Some(self.min_us.map(|m| m.min(latency_us)).unwrap_or(latency_us));
        self.max_us = Some(self.max_us.map(|m| m.max(latency_us)).unwrap_or(latency_us));
    }

    fn avg_us(&self) -> Option<f64> {
        if self.samples.is_empty() {
            return None;
        }
        Some(self.samples.iter().sum::<u64>() as f64 / self.samples.len() as f64)
    }
}

/// Test for measuring input latency
pub struct LatencyTest {
    /// Per-key latency measurements (based on delta from event)
    key_stats: HashMap<KeyCode, KeyLatencyStats>,
    /// Global latency samples
    global_samples: Vec<u64>,
    /// Last event timestamp (for consecutive key latency)
    last_event_time: Option<Instant>,
    /// Total events processed
    total_events: u64,
    /// Test start time
    start_time: Option<Instant>,
    /// Global min latency
    global_min_us: Option<u64>,
    /// Global max latency
    global_max_us: Option<u64>,
}

impl LatencyTest {
    pub fn new() -> Self {
        Self {
            key_stats: HashMap::new(),
            global_samples: Vec::with_capacity(10000),
            last_event_time: None,
            total_events: 0,
            start_time: None,
            global_min_us: None,
            global_max_us: None,
        }
    }

    /// Get global average latency in microseconds
    pub fn global_avg_us(&self) -> Option<f64> {
        if self.global_samples.is_empty() {
            return None;
        }
        Some(self.global_samples.iter().sum::<u64>() as f64 / self.global_samples.len() as f64)
    }

    /// Get global average latency in milliseconds
    pub fn global_avg_ms(&self) -> Option<f64> {
        self.global_avg_us().map(|us| us / 1000.0)
    }

    /// Get latency rating based on average
    pub fn latency_rating(&self) -> &'static str {
        match self.global_avg_ms() {
            None => "Not measured",
            Some(ms) if ms < 5.0 => "Excellent (<5ms)",
            Some(ms) if ms < 10.0 => "Great (<10ms)",
            Some(ms) if ms < 20.0 => "Good (<20ms)",
            Some(ms) if ms < 50.0 => "Acceptable (<50ms)",
            Some(_) => "Poor (>50ms)",
        }
    }

    /// Get standard deviation of latency
    pub fn std_dev_us(&self) -> Option<f64> {
        if self.global_samples.len() < 2 {
            return None;
        }
        let mean = self.global_avg_us()?;
        let variance = self.global_samples.iter()
            .map(|&x| {
                let diff = x as f64 - mean;
                diff * diff
            })
            .sum::<f64>() / self.global_samples.len() as f64;
        Some(variance.sqrt())
    }

    /// Get the key with highest latency
    pub fn slowest_key(&self) -> Option<(KeyCode, f64)> {
        self.key_stats
            .iter()
            .filter_map(|(k, stats)| stats.avg_us().map(|avg| (*k, avg)))
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    }

    /// Get the key with lowest latency
    pub fn fastest_key(&self) -> Option<(KeyCode, f64)> {
        self.key_stats
            .iter()
            .filter_map(|(k, stats)| stats.avg_us().map(|avg| (*k, avg)))
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    }
}

impl Default for LatencyTest {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyboardTest for LatencyTest {
    fn name(&self) -> &'static str {
        "Latency Test"
    }

    fn description(&self) -> &'static str {
        "Measures input latency for each key"
    }

    fn process_event(&mut self, event: &KeyEvent) {
        if self.start_time.is_none() {
            self.start_time = Some(Instant::now());
        }

        // Only measure key presses for latency
        if event.event_type != KeyEventType::Press {
            return;
        }

        self.total_events += 1;

        // Use delta_us from the event as our latency measurement
        // This represents the time since the last poll
        let latency_us = event.delta_us;

        // Record global sample
        if latency_us < 1_000_000 { // Ignore >1s gaps
            self.global_samples.push(latency_us);
            self.global_min_us = Some(
                self.global_min_us.map(|m| m.min(latency_us)).unwrap_or(latency_us)
            );
            self.global_max_us = Some(
                self.global_max_us.map(|m| m.max(latency_us)).unwrap_or(latency_us)
            );

            // Record per-key sample
            self.key_stats
                .entry(event.key)
                .or_default()
                .add_sample(latency_us);
        }

        self.last_event_time = Some(event.timestamp);
    }

    fn is_complete(&self) -> bool {
        // This test runs continuously
        false
    }

    fn get_results(&self) -> Vec<TestResult> {
        let mut results = Vec::new();

        // Tooltip: Explain what this test measures
        results.push(TestResult::info(
            "--- What This Measures ---",
            "",
        ));
        results.push(TestResult::info(
            "Input latency = time from",
            "keypress to PC detection",
        ));
        results.push(TestResult::info(
            "Affected by: polling rate,",
            "USB, debounce, drivers",
        ));
        results.push(TestResult::info(
            "Look for: <10ms excellent,",
            "<20ms good, >50ms poor",
        ));
        results.push(TestResult::info("", ""));

        results.push(TestResult::info(
            "Samples",
            format!("{}", self.global_samples.len()),
        ));

        // Average latency
        if let Some(avg_ms) = self.global_avg_ms() {
            let status = if avg_ms < 10.0 {
                ResultStatus::Ok
            } else if avg_ms < 20.0 {
                ResultStatus::Warning
            } else {
                ResultStatus::Error
            };
            results.push(TestResult::new(
                "Avg Latency",
                format!("{:.2} ms", avg_ms),
                status,
            ));
        }

        // Min/Max
        if let Some(min) = self.global_min_us {
            results.push(TestResult::info(
                "Min Latency",
                format!("{:.2} ms", min as f64 / 1000.0),
            ));
        }
        if let Some(max) = self.global_max_us {
            results.push(TestResult::info(
                "Max Latency",
                format!("{:.2} ms", max as f64 / 1000.0),
            ));
        }

        // Standard deviation
        if let Some(std_dev) = self.std_dev_us() {
            results.push(TestResult::info(
                "Std Dev",
                format!("{:.2} ms", std_dev / 1000.0),
            ));
        }

        // Rating
        results.push(TestResult::info(
            "Rating",
            self.latency_rating().to_string(),
        ));

        // Fastest/Slowest keys
        if let Some((key, latency)) = self.fastest_key() {
            let key_info = keymap::get_key_info(key);
            results.push(TestResult::ok(
                "Fastest Key",
                format!("{}: {:.2}ms", key_info.name, latency / 1000.0),
            ));
        }
        if let Some((key, latency)) = self.slowest_key() {
            let key_info = keymap::get_key_info(key);
            results.push(TestResult::warning(
                "Slowest Key",
                format!("{}: {:.2}ms", key_info.name, latency / 1000.0),
            ));
        }

        results
    }

    fn reset(&mut self) {
        self.key_stats.clear();
        self.global_samples.clear();
        self.last_event_time = None;
        self.total_events = 0;
        self.start_time = None;
        self.global_min_us = None;
        self.global_max_us = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_press_event(key: KeyCode, delta_us: u64) -> KeyEvent {
        KeyEvent {
            key,
            event_type: KeyEventType::Press,
            timestamp: Instant::now(),
            delta_us,
        }
    }

    fn make_release_event(key: KeyCode) -> KeyEvent {
        KeyEvent {
            key,
            event_type: KeyEventType::Release,
            timestamp: Instant::now(),
            delta_us: 0,
        }
    }

    #[test]
    fn new_test_initial_state() {
        let test = LatencyTest::new();
        assert!(test.global_avg_us().is_none());
        assert!(test.global_avg_ms().is_none());
        assert!(test.std_dev_us().is_none());
        assert!(test.slowest_key().is_none());
        assert!(test.fastest_key().is_none());
    }

    #[test]
    fn latency_rating_not_measured() {
        let test = LatencyTest::new();
        assert_eq!(test.latency_rating(), "Not measured");
    }

    #[test]
    fn latency_rating_excellent() {
        let mut test = LatencyTest::new();
        // 4ms = 4000us
        test.global_samples = vec![4000, 4000, 4000];
        assert_eq!(test.latency_rating(), "Excellent (<5ms)");
    }

    #[test]
    fn latency_rating_great() {
        let mut test = LatencyTest::new();
        // 8ms = 8000us
        test.global_samples = vec![8000, 8000, 8000];
        assert_eq!(test.latency_rating(), "Great (<10ms)");
    }

    #[test]
    fn latency_rating_good() {
        let mut test = LatencyTest::new();
        // 15ms = 15000us
        test.global_samples = vec![15000, 15000, 15000];
        assert_eq!(test.latency_rating(), "Good (<20ms)");
    }

    #[test]
    fn latency_rating_acceptable() {
        let mut test = LatencyTest::new();
        // 30ms = 30000us
        test.global_samples = vec![30000, 30000, 30000];
        assert_eq!(test.latency_rating(), "Acceptable (<50ms)");
    }

    #[test]
    fn latency_rating_poor() {
        let mut test = LatencyTest::new();
        // 60ms = 60000us
        test.global_samples = vec![60000, 60000, 60000];
        assert_eq!(test.latency_rating(), "Poor (>50ms)");
    }

    #[test]
    fn global_avg_calculation() {
        let mut test = LatencyTest::new();
        test.global_samples = vec![1000, 2000, 3000]; // avg = 2000us

        let avg_us = test.global_avg_us().unwrap();
        assert!((avg_us - 2000.0).abs() < 0.01);

        let avg_ms = test.global_avg_ms().unwrap();
        assert!((avg_ms - 2.0).abs() < 0.01);
    }

    #[test]
    fn std_dev_calculation() {
        let mut test = LatencyTest::new();
        // [1000, 2000, 3000] - mean = 2000, variance = ((1000)^2 + 0 + (1000)^2) / 3
        test.global_samples = vec![1000, 2000, 3000];

        let std_dev = test.std_dev_us().unwrap();
        // Expected: sqrt((1000000 + 0 + 1000000) / 3) ≈ 816.5
        assert!(std_dev > 800.0 && std_dev < 850.0);
    }

    #[test]
    fn std_dev_requires_two_samples() {
        let mut test = LatencyTest::new();
        test.global_samples = vec![1000];
        assert!(test.std_dev_us().is_none());
    }

    #[test]
    fn process_event_ignores_release() {
        let mut test = LatencyTest::new();
        test.process_event(&make_release_event(KeyCode(30)));

        assert_eq!(test.total_events, 0);
        assert!(test.global_samples.is_empty());
    }

    #[test]
    fn process_event_records_latency() {
        let mut test = LatencyTest::new();
        test.process_event(&make_press_event(KeyCode(30), 5000)); // 5ms

        assert_eq!(test.total_events, 1);
        assert_eq!(test.global_samples.len(), 1);
        assert_eq!(test.global_samples[0], 5000);
        assert_eq!(test.global_min_us, Some(5000));
        assert_eq!(test.global_max_us, Some(5000));
    }

    #[test]
    fn process_event_filters_large_latency() {
        let mut test = LatencyTest::new();
        // >1 second gaps should be filtered
        test.process_event(&make_press_event(KeyCode(30), 2_000_000));

        assert_eq!(test.total_events, 1);
        assert!(test.global_samples.is_empty());
    }

    #[test]
    fn min_max_tracking() {
        let mut test = LatencyTest::new();
        test.process_event(&make_press_event(KeyCode(30), 5000));
        test.process_event(&make_press_event(KeyCode(31), 2000));
        test.process_event(&make_press_event(KeyCode(32), 8000));

        assert_eq!(test.global_min_us, Some(2000));
        assert_eq!(test.global_max_us, Some(8000));
    }

    #[test]
    fn fastest_slowest_key() {
        let mut test = LatencyTest::new();
        // Key 30: 2ms, Key 31: 5ms, Key 32: 3ms
        test.process_event(&make_press_event(KeyCode(30), 2000));
        test.process_event(&make_press_event(KeyCode(31), 5000));
        test.process_event(&make_press_event(KeyCode(32), 3000));

        let (fastest_key, fastest_latency) = test.fastest_key().unwrap();
        assert_eq!(fastest_key, KeyCode(30));
        assert!((fastest_latency - 2000.0).abs() < 0.01);

        let (slowest_key, slowest_latency) = test.slowest_key().unwrap();
        assert_eq!(slowest_key, KeyCode(31));
        assert!((slowest_latency - 5000.0).abs() < 0.01);
    }

    #[test]
    fn reset_clears_all() {
        let mut test = LatencyTest::new();
        test.process_event(&make_press_event(KeyCode(30), 5000));
        test.process_event(&make_press_event(KeyCode(31), 3000));

        test.reset();

        assert!(test.global_samples.is_empty());
        assert!(test.key_stats.is_empty());
        assert_eq!(test.total_events, 0);
        assert!(test.global_min_us.is_none());
        assert!(test.global_max_us.is_none());
    }

    #[test]
    fn test_name_and_description() {
        let test = LatencyTest::new();
        assert_eq!(test.name(), "Latency Test");
        assert!(!test.description().is_empty());
    }

    #[test]
    fn is_never_complete() {
        let mut test = LatencyTest::new();
        test.process_event(&make_press_event(KeyCode(30), 5000));
        assert!(!test.is_complete()); // Continuous test
    }

    // KeyLatencyStats tests
    #[test]
    fn key_latency_stats_add_sample() {
        let mut stats = KeyLatencyStats::default();
        stats.add_sample(1000);
        stats.add_sample(2000);
        stats.add_sample(500);

        assert_eq!(stats.samples.len(), 3);
        assert_eq!(stats.min_us, Some(500));
        assert_eq!(stats.max_us, Some(2000));
    }

    #[test]
    fn key_latency_stats_avg() {
        let mut stats = KeyLatencyStats::default();
        stats.add_sample(1000);
        stats.add_sample(2000);
        stats.add_sample(3000);

        let avg = stats.avg_us().unwrap();
        assert!((avg - 2000.0).abs() < 0.01);
    }
}
