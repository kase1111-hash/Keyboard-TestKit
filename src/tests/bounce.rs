//! Hold down and release test with bounce detection

use super::{KeyboardTest, TestResult, ResultStatus};
use crate::keyboard::{KeyCode, KeyEvent, KeyEventType, keymap};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Record of key press/release events for bounce analysis
#[derive(Debug, Clone)]
struct KeyEventRecord {
    event_type: KeyEventType,
    timestamp: Instant,
}

/// Statistics for a single key's hold/release behavior
#[derive(Debug, Clone, Default)]
struct KeyHoldStats {
    /// All events for this key (for bounce detection)
    events: Vec<KeyEventRecord>,
    /// Number of detected bounces
    bounce_count: u32,
    /// Total press count
    press_count: u32,
    /// Minimum hold duration
    min_hold_ms: Option<f64>,
    /// Maximum hold duration
    max_hold_ms: Option<f64>,
    /// Sum of hold durations for averaging
    total_hold_ms: f64,
    /// Currently pressed
    is_pressed: bool,
    /// Current press start time
    press_start: Option<Instant>,
}

/// Test for hold down, release, and bounce detection
pub struct HoldReleaseTest {
    /// Per-key statistics
    key_stats: HashMap<KeyCode, KeyHoldStats>,
    /// Bounce detection window (events within this time are considered bounces)
    bounce_window: Duration,
    /// Total bounces detected across all keys
    total_bounces: u32,
    /// Total key presses
    total_presses: u32,
    /// Currently held keys for display
    held_keys: Vec<(KeyCode, Instant)>,
    /// Test start time
    start_time: Option<Instant>,
    /// Last event for repeat rate detection
    last_event_time: Option<Instant>,
    /// Repeat events detected (same key pressed rapidly)
    repeat_intervals: Vec<u64>,
}

impl HoldReleaseTest {
    pub fn new(bounce_window_ms: u64) -> Self {
        Self {
            key_stats: HashMap::new(),
            bounce_window: Duration::from_millis(bounce_window_ms),
            total_bounces: 0,
            total_presses: 0,
            held_keys: Vec::new(),
            start_time: None,
            last_event_time: None,
            repeat_intervals: Vec::new(),
        }
    }

    /// Check if an event constitutes a bounce (static version to avoid borrow issues)
    fn check_bounce(stats: &KeyHoldStats, event: &KeyEvent, bounce_window: Duration) -> bool {
        if stats.events.is_empty() {
            return false;
        }

        // Get the last event of opposite type
        let last_opposite = stats.events.iter().rev().find(|e| e.event_type != event.event_type);

        if let Some(last) = last_opposite {
            let time_since = event.timestamp.duration_since(last.timestamp);
            // If the opposite event happened within bounce window, it's a bounce
            time_since < bounce_window
        } else {
            false
        }
    }

    /// Calculate average hold duration in ms
    pub fn avg_hold_ms(&self) -> Option<f64> {
        if self.total_presses == 0 {
            return None;
        }
        let total: f64 = self.key_stats.values().map(|s| s.total_hold_ms).sum();
        Some(total / self.total_presses as f64)
    }

    /// Calculate repeat rate in keys per second
    pub fn repeat_rate_hz(&self) -> Option<f64> {
        if self.repeat_intervals.len() < 2 {
            return None;
        }
        let avg_interval_us = self.repeat_intervals.iter().sum::<u64>() as f64
            / self.repeat_intervals.len() as f64;
        if avg_interval_us > 0.0 {
            Some(1_000_000.0 / avg_interval_us)
        } else {
            None
        }
    }

    /// Get keys with bounces
    pub fn bouncy_keys(&self) -> Vec<(KeyCode, u32)> {
        self.key_stats
            .iter()
            .filter(|(_, stats)| stats.bounce_count > 0)
            .map(|(key, stats)| (*key, stats.bounce_count))
            .collect()
    }

    /// Get currently held keys with durations
    pub fn held_keys(&self) -> Vec<(KeyCode, Duration)> {
        let now = Instant::now();
        self.held_keys
            .iter()
            .map(|(key, start)| (*key, now.duration_since(*start)))
            .collect()
    }
}

impl Default for HoldReleaseTest {
    fn default() -> Self {
        Self::new(5) // 5ms default bounce window
    }
}

impl KeyboardTest for HoldReleaseTest {
    fn name(&self) -> &'static str {
        "Hold & Release Test"
    }

    fn description(&self) -> &'static str {
        "Tests key hold duration, release behavior, and detects key bounce"
    }

    fn process_event(&mut self, event: &KeyEvent) {
        if self.start_time.is_none() {
            self.start_time = Some(Instant::now());
        }

        let bounce_window = self.bounce_window;
        let stats = self.key_stats.entry(event.key).or_default();

        // Check for bounce before recording
        let is_bounce = Self::check_bounce(stats, event, bounce_window);
        if is_bounce {
            stats.bounce_count += 1;
            self.total_bounces += 1;
        }

        // Record the event
        stats.events.push(KeyEventRecord {
            event_type: event.event_type,
            timestamp: event.timestamp,
        });

        // Keep only last 100 events per key to save memory
        if stats.events.len() > 100 {
            stats.events.remove(0);
        }

        match event.event_type {
            KeyEventType::Press => {
                stats.is_pressed = true;
                stats.press_start = Some(event.timestamp);
                stats.press_count += 1;
                self.total_presses += 1;

                // Track held keys
                if !self.held_keys.iter().any(|(k, _)| *k == event.key) {
                    self.held_keys.push((event.key, event.timestamp));
                }

                // Track repeat intervals
                if let Some(last) = self.last_event_time {
                    let interval = event.timestamp.duration_since(last).as_micros() as u64;
                    if interval < 500_000 { // Only track intervals < 500ms
                        self.repeat_intervals.push(interval);
                        if self.repeat_intervals.len() > 100 {
                            self.repeat_intervals.remove(0);
                        }
                    }
                }
                self.last_event_time = Some(event.timestamp);
            }
            KeyEventType::Release => {
                stats.is_pressed = false;

                // Calculate hold duration
                if let Some(start) = stats.press_start.take() {
                    let hold_ms = event.timestamp.duration_since(start).as_secs_f64() * 1000.0;
                    stats.total_hold_ms += hold_ms;

                    stats.min_hold_ms = Some(
                        stats.min_hold_ms.map(|m| m.min(hold_ms)).unwrap_or(hold_ms)
                    );
                    stats.max_hold_ms = Some(
                        stats.max_hold_ms.map(|m| m.max(hold_ms)).unwrap_or(hold_ms)
                    );
                }

                // Remove from held keys
                self.held_keys.retain(|(k, _)| *k != event.key);
            }
        }
    }

    fn is_complete(&self) -> bool {
        false // Continuous test
    }

    fn get_results(&self) -> Vec<TestResult> {
        let mut results = Vec::new();

        // Tooltip: Explain what this test measures
        results.push(TestResult::info(
            "--- What This Measures ---",
            "",
        ));
        results.push(TestResult::info(
            "Tests key hold/release and",
            "detects switch bounce",
        ));
        results.push(TestResult::info(
            "Bounce = false triggers from",
            "switch contact vibration",
        ));
        results.push(TestResult::info(
            "Look for: 0 bounces, stable",
            "hold times, clean releases",
        ));
        results.push(TestResult::info("", ""));

        results.push(TestResult::info(
            "Total Presses",
            format!("{}", self.total_presses),
        ));

        // Bounce detection results
        if self.total_bounces == 0 {
            results.push(TestResult::ok("Bounces Detected", "None"));
        } else {
            results.push(TestResult::error(
                "Bounces Detected",
                format!("{}", self.total_bounces),
            ));
        }

        results.push(TestResult::info(
            "Bounce Window",
            format!("{} ms", self.bounce_window.as_millis()),
        ));

        // Hold duration stats
        if let Some(avg) = self.avg_hold_ms() {
            results.push(TestResult::info(
                "Avg Hold Time",
                format!("{:.1} ms", avg),
            ));
        }

        // Repeat rate
        if let Some(rate) = self.repeat_rate_hz() {
            results.push(TestResult::info(
                "Repeat Rate",
                format!("{:.1} keys/sec", rate),
            ));
        }

        // Bouncy keys
        let bouncy = self.bouncy_keys();
        if !bouncy.is_empty() {
            results.push(TestResult::warning("--- Bouncy Keys ---", ""));
            for (key, count) in bouncy.iter().take(5) {
                let key_info = keymap::get_key_info(*key);
                results.push(TestResult::error(
                    format!("  {}", key_info.name),
                    format!("{} bounces", count),
                ));
            }
        }

        // Currently held keys
        let held = self.held_keys();
        if !held.is_empty() {
            results.push(TestResult::info("--- Held Keys ---", ""));
            for (key, duration) in held.iter().take(5) {
                let key_info = keymap::get_key_info(*key);
                results.push(TestResult::info(
                    format!("  {}", key_info.name),
                    format!("{:.2}s", duration.as_secs_f64()),
                ));
            }
        }

        results
    }

    fn reset(&mut self) {
        self.key_stats.clear();
        self.total_bounces = 0;
        self.total_presses = 0;
        self.held_keys.clear();
        self.start_time = None;
        self.last_event_time = None;
        self.repeat_intervals.clear();
    }
}
