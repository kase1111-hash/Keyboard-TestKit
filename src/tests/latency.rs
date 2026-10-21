//! Event timing measurement test module
//!
//! Records the gap between each key press and the key event before it (any
//! key). Between different keys that gap is set by the typist (50-300 ms when
//! typing), not by the keyboard, so the results are informational statistics
//! and never rate the keyboard. The keyboard's own report interval is
//! estimated by the polling test from timestamp quantization; true
//! end-to-end input latency requires external hardware measurement.

use super::{KeyboardTest, TestResult};
use crate::keyboard::{keymap, KeyCode, KeyEvent, KeyEventType};
use crate::utils::MinMaxExt;
use std::collections::HashMap;
use std::time::Instant;

/// Per-key timing statistics
#[derive(Debug, Clone, Default)]
struct KeyTimingStats {
    samples: Vec<u64>,
    min_us: Option<u64>,
    max_us: Option<u64>,
}

impl KeyTimingStats {
    fn add_sample(&mut self, timing_us: u64) {
        self.samples.push(timing_us);
        self.min_us.update_min(timing_us);
        self.max_us.update_max(timing_us);
    }

    fn avg_us(&self) -> Option<f64> {
        if self.samples.is_empty() {
            return None;
        }
        Some(self.samples.iter().sum::<u64>() as f64 / self.samples.len() as f64)
    }
}

/// Test for measuring inter-event timing
///
/// Measures the time between each key press and the previous key event. This
/// reflects typing rhythm, not keyboard speed or latency, so nothing here is
/// colour-coded as good or bad.
pub struct EventTimingTest {
    /// Per-key timing measurements (based on delta from event)
    key_stats: HashMap<KeyCode, KeyTimingStats>,
    /// Global timing samples
    global_samples: Vec<u64>,
    /// Last event timestamp (for consecutive key timing)
    last_event_time: Option<Instant>,
    /// Total events processed
    total_events: u64,
    /// Test start time
    start_time: Option<Instant>,
    /// Global min timing
    global_min_us: Option<u64>,
    /// Global max timing
    global_max_us: Option<u64>,
}

impl EventTimingTest {
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

    /// Get global average timing in microseconds
    pub fn global_avg_us(&self) -> Option<f64> {
        if self.global_samples.is_empty() {
            return None;
        }
        Some(self.global_samples.iter().sum::<u64>() as f64 / self.global_samples.len() as f64)
    }

    /// Get global average timing in milliseconds
    pub fn global_avg_ms(&self) -> Option<f64> {
        self.global_avg_us().map(|us| us / 1000.0)
    }

    /// Describe what the average gap means.
    ///
    /// Deliberately not a quality rating: the gaps between key events are set
    /// by the typist, so a "poor" value would only say how fast someone typed.
    pub fn timing_rating(&self) -> &'static str {
        match self.global_avg_ms() {
            None => "Not measured",
            Some(_) => "Typing rhythm (not a keyboard rating)",
        }
    }

    /// Get standard deviation of timing
    pub fn std_dev_us(&self) -> Option<f64> {
        if self.global_samples.len() < 2 {
            return None;
        }
        let mean = self.global_avg_us()?;
        let variance = self
            .global_samples
            .iter()
            .map(|&x| {
                let diff = x as f64 - mean;
                diff * diff
            })
            .sum::<f64>()
            / self.global_samples.len() as f64;
        Some(variance.sqrt())
    }

    /// Get the key with highest timing (slowest)
    pub fn slowest_key(&self) -> Option<(KeyCode, f64)> {
        self.key_stats
            .iter()
            .filter_map(|(k, stats)| stats.avg_us().map(|avg| (*k, avg)))
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    }

    /// Get the key with lowest timing (fastest)
    pub fn fastest_key(&self) -> Option<(KeyCode, f64)> {
        self.key_stats
            .iter()
            .filter_map(|(k, stats)| stats.avg_us().map(|avg| (*k, avg)))
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    }
}

impl Default for EventTimingTest {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyboardTest for EventTimingTest {
    fn name(&self) -> &'static str {
        "Event Timing"
    }

    fn description(&self) -> &'static str {
        "Measures inter-event timing (gap before each key press; informational)"
    }

    fn process_event(&mut self, event: &KeyEvent) {
        if self.start_time.is_none() {
            self.start_time = Some(Instant::now());
        }

        // Only measure key presses for timing
        if event.event_type != KeyEventType::Press {
            return;
        }

        self.total_events += 1;

        // delta_us is the time since the previous key event of any key
        let timing_us = event.delta_us;

        // Record global sample
        if timing_us < 1_000_000 {
            // Ignore >1s gaps
            self.global_samples.push(timing_us);
            self.global_min_us.update_min(timing_us);
            self.global_max_us.update_max(timing_us);

            // Record per-key sample
            self.key_stats
                .entry(event.key)
                .or_default()
                .add_sample(timing_us);
        }

        self.last_event_time = Some(event.timestamp);
    }

    fn is_complete(&self) -> bool {
        false
    }

    fn get_results(&self) -> Vec<TestResult> {
        let mut results = Vec::new();

        results.push(TestResult::info("--- What This Measures ---", ""));
        results.push(TestResult::info(
            "Gap before each press",
            "since the previous key event",
        ));
        results.push(TestResult::info(
            "Set by your typing rhythm,",
            "not by the keyboard",
        ));
        results.push(TestResult::info(
            "Keyboard report interval:",
            "see Polling view (Est. Poll Rate)",
        ));
        results.push(TestResult::info(
            "NOT true input latency",
            "(requires hardware probe)",
        ));
        results.push(TestResult::info("", ""));

        results.push(TestResult::info(
            "Samples",
            format!("{}", self.global_samples.len()),
        ));

        // Statistics only: none of these say anything about keyboard quality.
        if let Some(avg_ms) = self.global_avg_ms() {
            results.push(TestResult::info("Avg Gap", format!("{:.2} ms", avg_ms)));
        }
        if let Some(min) = self.global_min_us {
            // 0 ms is normal: keys sent in the same USB report share a timestamp
            results.push(TestResult::info(
                "Shortest Gap",
                format!("{:.2} ms", min as f64 / 1000.0),
            ));
        }
        if let Some(max) = self.global_max_us {
            results.push(TestResult::info(
                "Longest Gap",
                format!("{:.2} ms", max as f64 / 1000.0),
            ));
        }
        if let Some(std_dev) = self.std_dev_us() {
            results.push(TestResult::info(
                "Std Dev",
                format!("{:.2} ms", std_dev / 1000.0),
            ));
        }

        // Per-key averages show which keys you reach quickly, not slow keys
        if let Some((key, timing)) = self.fastest_key() {
            let key_info = keymap::get_key_info(key);
            results.push(TestResult::info(
                "Shortest Avg Gap",
                format!("{}: {:.2} ms", key_info.name, timing / 1000.0),
            ));
        }
        if let Some((key, timing)) = self.slowest_key() {
            let key_info = keymap::get_key_info(key);
            results.push(TestResult::info(
                "Longest Avg Gap",
                format!("{}: {:.2} ms", key_info.name, timing / 1000.0),
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
    use crate::tests::test_helpers::{make_press, release};

    #[test]
    fn new_test_initial_state() {
        let test = EventTimingTest::new();
        assert!(test.global_avg_us().is_none());
        assert!(test.global_avg_ms().is_none());
        assert!(test.std_dev_us().is_none());
        assert!(test.slowest_key().is_none());
        assert!(test.fastest_key().is_none());
    }

    #[test]
    fn timing_rating_not_measured() {
        let test = EventTimingTest::new();
        assert_eq!(test.timing_rating(), "Not measured");
    }

    #[test]
    fn timing_rating_is_not_a_quality_verdict() {
        // Gaps of 4 ms or 60 ms only describe how someone typed
        for gap in [4000u64, 15000, 60000, 150000] {
            let mut test = EventTimingTest::new();
            test.global_samples = vec![gap; 3];
            let rating = test.timing_rating();
            for verdict in ["Excellent", "Great", "Good", "Acceptable", "Poor"] {
                assert!(!rating.contains(verdict), "{}", rating);
            }
        }
    }

    #[test]
    fn normal_typing_is_not_rated_as_a_slow_keyboard() {
        // A healthy 1000 Hz keyboard under a normal typist: 50-300 ms gaps,
        // plus two keys that arrived in the same USB report (0 ms).
        let mut test = EventTimingTest::new();
        for (i, gap) in [90_000u64, 120_000, 0, 75_000, 280_000, 150_000]
            .iter()
            .enumerate()
        {
            test.process_event(&make_press(KeyCode(30 + i as u16), *gap));
        }
        let results = test.get_results();
        let judged: Vec<&TestResult> = results
            .iter()
            .filter(|r| r.status != crate::tests::ResultStatus::Info)
            .collect();
        assert!(judged.is_empty(), "{:?}", judged);
        assert!(!results.iter().any(|r| r.value.contains("Poor")));
        assert!(results
            .iter()
            .any(|r| r.label == "Avg Gap" && r.value.starts_with("119.17")));
        // Points to the real keyboard measurement
        assert!(results.iter().any(|r| r.value.contains("Polling view")));
    }

    #[test]
    fn global_avg_calculation() {
        let mut test = EventTimingTest::new();
        test.global_samples = vec![1000, 2000, 3000];

        let avg_us = test.global_avg_us().unwrap();
        assert!((avg_us - 2000.0).abs() < 0.01);

        let avg_ms = test.global_avg_ms().unwrap();
        assert!((avg_ms - 2.0).abs() < 0.01);
    }

    #[test]
    fn std_dev_calculation() {
        let mut test = EventTimingTest::new();
        test.global_samples = vec![1000, 2000, 3000];

        let std_dev = test.std_dev_us().unwrap();
        assert!(std_dev > 800.0 && std_dev < 850.0);
    }

    #[test]
    fn std_dev_requires_two_samples() {
        let mut test = EventTimingTest::new();
        test.global_samples = vec![1000];
        assert!(test.std_dev_us().is_none());
    }

    #[test]
    fn process_event_ignores_release() {
        let mut test = EventTimingTest::new();
        test.process_event(&release(KeyCode(30)));

        assert_eq!(test.total_events, 0);
        assert!(test.global_samples.is_empty());
    }

    #[test]
    fn process_event_records_timing() {
        let mut test = EventTimingTest::new();
        test.process_event(&make_press(KeyCode(30), 5000));

        assert_eq!(test.total_events, 1);
        assert_eq!(test.global_samples.len(), 1);
        assert_eq!(test.global_samples[0], 5000);
        assert_eq!(test.global_min_us, Some(5000));
        assert_eq!(test.global_max_us, Some(5000));
    }

    #[test]
    fn process_event_filters_large_gaps() {
        let mut test = EventTimingTest::new();
        test.process_event(&make_press(KeyCode(30), 2_000_000));

        assert_eq!(test.total_events, 1);
        assert!(test.global_samples.is_empty());
    }

    #[test]
    fn min_max_tracking() {
        let mut test = EventTimingTest::new();
        test.process_event(&make_press(KeyCode(30), 5000));
        test.process_event(&make_press(KeyCode(31), 2000));
        test.process_event(&make_press(KeyCode(32), 8000));

        assert_eq!(test.global_min_us, Some(2000));
        assert_eq!(test.global_max_us, Some(8000));
    }

    #[test]
    fn fastest_slowest_key() {
        let mut test = EventTimingTest::new();
        test.process_event(&make_press(KeyCode(30), 2000));
        test.process_event(&make_press(KeyCode(31), 5000));
        test.process_event(&make_press(KeyCode(32), 3000));

        let (fastest_key, fastest_timing) = test.fastest_key().unwrap();
        assert_eq!(fastest_key, KeyCode(30));
        assert!((fastest_timing - 2000.0).abs() < 0.01);

        let (slowest_key, slowest_timing) = test.slowest_key().unwrap();
        assert_eq!(slowest_key, KeyCode(31));
        assert!((slowest_timing - 5000.0).abs() < 0.01);
    }

    #[test]
    fn reset_clears_all() {
        let mut test = EventTimingTest::new();
        test.process_event(&make_press(KeyCode(30), 5000));
        test.process_event(&make_press(KeyCode(31), 3000));

        test.reset();

        assert!(test.global_samples.is_empty());
        assert!(test.key_stats.is_empty());
        assert_eq!(test.total_events, 0);
        assert!(test.global_min_us.is_none());
        assert!(test.global_max_us.is_none());
    }

    #[test]
    fn test_name_and_description() {
        let test = EventTimingTest::new();
        assert_eq!(test.name(), "Event Timing");
        assert!(test.description().contains("inter-event"));
    }

    #[test]
    fn is_never_complete() {
        let mut test = EventTimingTest::new();
        test.process_event(&make_press(KeyCode(30), 5000));
        assert!(!test.is_complete());
    }

    #[test]
    fn key_timing_stats_add_sample() {
        let mut stats = KeyTimingStats::default();
        stats.add_sample(1000);
        stats.add_sample(2000);
        stats.add_sample(500);

        assert_eq!(stats.samples.len(), 3);
        assert_eq!(stats.min_us, Some(500));
        assert_eq!(stats.max_us, Some(2000));
    }

    #[test]
    fn key_timing_stats_avg() {
        let mut stats = KeyTimingStats::default();
        stats.add_sample(1000);
        stats.add_sample(2000);
        stats.add_sample(3000);

        let avg = stats.avg_us().unwrap();
        assert!((avg - 2000.0).abs() < 0.01);
    }
}
