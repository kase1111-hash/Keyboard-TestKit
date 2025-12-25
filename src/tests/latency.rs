//! Latency measurement test module

use super::{KeyboardTest, TestResult, ResultStatus};
use crate::keyboard::{KeyCode, KeyEvent, KeyEventType, keymap};
use std::collections::HashMap;
use std::time::{Duration, Instant};

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
