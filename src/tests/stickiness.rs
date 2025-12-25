//! Stickiness detection test module

use super::{KeyboardTest, TestResult, ResultStatus};
use crate::keyboard::{KeyCode, KeyEvent, KeyEventType, keymap};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Record of a potentially sticky key
#[derive(Debug, Clone)]
struct StickyKeyRecord {
    key: KeyCode,
    press_time: Instant,
    duration_at_detection: Duration,
    occurrences: u32,
}

/// Test for detecting stuck or sticky keys
pub struct StickinessTest {
    /// Threshold duration after which a held key is flagged
    threshold: Duration,
    /// Currently held keys with their press times
    held_keys: HashMap<KeyCode, Instant>,
    /// Keys that have been flagged as potentially stuck
    flagged_keys: Vec<StickyKeyRecord>,
    /// Keys currently flagged (to avoid duplicate alerts)
    currently_flagged: HashMap<KeyCode, Instant>,
    /// Total keys tested
    keys_tested: u32,
    /// Test start time
    start_time: Option<Instant>,
}

impl StickinessTest {
    pub fn new(threshold_ms: u64) -> Self {
        Self {
            threshold: Duration::from_millis(threshold_ms),
            held_keys: HashMap::new(),
            flagged_keys: Vec::new(),
            currently_flagged: HashMap::new(),
            keys_tested: 0,
            start_time: None,
        }
    }

    /// Check all held keys for stickiness
    pub fn check_stuck_keys(&mut self) -> Vec<KeyCode> {
        let now = Instant::now();
        let mut newly_stuck = Vec::new();

        for (&key, &press_time) in &self.held_keys {
            let duration = now.duration_since(press_time);

            if duration > self.threshold && !self.currently_flagged.contains_key(&key) {
                self.currently_flagged.insert(key, now);

                // Check if we already have a record for this key
                if let Some(record) = self.flagged_keys.iter_mut().find(|r| r.key == key) {
                    record.occurrences += 1;
                } else {
                    self.flagged_keys.push(StickyKeyRecord {
                        key,
                        press_time,
                        duration_at_detection: duration,
                        occurrences: 1,
                    });
                }

                newly_stuck.push(key);
            }
        }

        newly_stuck
    }

    /// Get currently held keys
    pub fn held_keys(&self) -> Vec<(KeyCode, Duration)> {
        let now = Instant::now();
        self.held_keys
            .iter()
            .map(|(&key, &press_time)| (key, now.duration_since(press_time)))
            .collect()
    }

    /// Get the threshold duration
    pub fn threshold(&self) -> Duration {
        self.threshold
    }

    /// Set a new threshold
    pub fn set_threshold(&mut self, threshold_ms: u64) {
        self.threshold = Duration::from_millis(threshold_ms);
    }
}

impl KeyboardTest for StickinessTest {
    fn name(&self) -> &'static str {
        "Stickiness Detection"
    }

    fn description(&self) -> &'static str {
        "Detects keys that remain pressed or fail to release properly"
    }

    fn process_event(&mut self, event: &KeyEvent) {
        if self.start_time.is_none() {
            self.start_time = Some(Instant::now());
        }

        match event.event_type {
            KeyEventType::Press => {
                self.held_keys.insert(event.key, event.timestamp);
                self.keys_tested += 1;
            }
            KeyEventType::Release => {
                self.held_keys.remove(&event.key);
                self.currently_flagged.remove(&event.key);
            }
        }
    }

    fn is_complete(&self) -> bool {
        // This test runs continuously
        false
    }

    fn get_results(&self) -> Vec<TestResult> {
        let mut results = Vec::new();

        results.push(TestResult::info(
            "Keys Tested",
            format!("{}", self.keys_tested),
        ));

        results.push(TestResult::info(
            "Currently Held",
            format!("{}", self.held_keys.len()),
        ));

        results.push(TestResult::info(
            "Threshold",
            format!("{} ms", self.threshold.as_millis()),
        ));

        if self.flagged_keys.is_empty() {
            results.push(TestResult::ok("Sticky Keys", "None detected"));
        } else {
            results.push(TestResult::warning(
                "Sticky Keys Found",
                format!("{}", self.flagged_keys.len()),
            ));

            for record in &self.flagged_keys {
                let key_info = keymap::get_key_info(record.key);
                results.push(TestResult::error(
                    format!("  {}", key_info.name),
                    format!("{} occurrences", record.occurrences),
                ));
            }
        }

        // Show currently held keys
        let held = self.held_keys();
        if !held.is_empty() {
            results.push(TestResult::info("--- Held Keys ---", ""));
            for (key, duration) in held {
                let key_info = keymap::get_key_info(key);
                let status = if duration > self.threshold {
                    ResultStatus::Warning
                } else {
                    ResultStatus::Info
                };
                results.push(TestResult::new(
                    format!("  {}", key_info.name),
                    format!("{:.1}s", duration.as_secs_f64()),
                    status,
                ));
            }
        }

        results
    }

    fn reset(&mut self) {
        self.held_keys.clear();
        self.flagged_keys.clear();
        self.currently_flagged.clear();
        self.keys_tested = 0;
        self.start_time = None;
    }
}
