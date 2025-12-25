//! N-Key Rollover and Ghosting test module

use super::{KeyboardTest, TestResult, ResultStatus};
use crate::keyboard::{KeyCode, KeyEvent, KeyEventType, keymap};
use std::collections::HashSet;
use std::time::Instant;

/// Test for N-Key Rollover and ghosting detection
pub struct RolloverTest {
    /// Currently pressed keys
    pressed_keys: HashSet<KeyCode>,
    /// Maximum simultaneous keys achieved
    max_simultaneous: usize,
    /// History of simultaneous key counts
    rollover_history: Vec<usize>,
    /// Detected ghost keys (keys registered without being pressed)
    ghost_detections: Vec<GhostEvent>,
    /// Total key events processed
    total_events: u64,
    /// Test start time
    start_time: Option<Instant>,
    /// Expected keys (for ghost detection - set by user)
    expected_keys: HashSet<KeyCode>,
}

/// A detected ghosting event
#[derive(Debug, Clone)]
struct GhostEvent {
    ghost_key: KeyCode,
    pressed_keys: Vec<KeyCode>,
    timestamp: Instant,
}

impl RolloverTest {
    pub fn new() -> Self {
        Self {
            pressed_keys: HashSet::new(),
            max_simultaneous: 0,
            rollover_history: Vec::new(),
            ghost_detections: Vec::new(),
            total_events: 0,
            start_time: None,
            expected_keys: HashSet::new(),
        }
    }

    /// Get current number of pressed keys
    pub fn current_count(&self) -> usize {
        self.pressed_keys.len()
    }

    /// Get maximum achieved rollover
    pub fn max_rollover(&self) -> usize {
        self.max_simultaneous
    }

    /// Get the rollover rating string
    pub fn rollover_rating(&self) -> String {
        match self.max_simultaneous {
            0 => "Not tested".to_string(),
            n if n >= 10 => "NKRO".to_string(),
            n => format!("{}KRO", n),
        }
    }

    /// Get currently pressed keys
    pub fn pressed_keys(&self) -> Vec<KeyCode> {
        self.pressed_keys.iter().copied().collect()
    }

    /// Check for potential ghosting
    /// Returns true if ghosting was detected
    fn check_ghosting(&mut self, new_key: KeyCode, timestamp: Instant) -> bool {
        // Simple ghosting detection: if we have 3+ keys and this key wasn't expected
        // Note: This is a simplified heuristic. Real ghosting detection would need
        // to understand the keyboard matrix layout.

        // For now, we track ghost detections when the user marks keys as unexpected
        // In practice, this would integrate with a visual UI where user can flag ghosts

        if self.pressed_keys.len() >= 3 && self.expected_keys.len() > 0 {
            if !self.expected_keys.contains(&new_key) {
                self.ghost_detections.push(GhostEvent {
                    ghost_key: new_key,
                    pressed_keys: self.pressed_keys.iter().copied().collect(),
                    timestamp,
                });
                return true;
            }
        }

        false
    }

    /// Get average rollover from history
    pub fn avg_rollover(&self) -> f64 {
        if self.rollover_history.is_empty() {
            return 0.0;
        }
        self.rollover_history.iter().sum::<usize>() as f64 / self.rollover_history.len() as f64
    }
}

impl Default for RolloverTest {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyboardTest for RolloverTest {
    fn name(&self) -> &'static str {
        "N-Key Rollover Test"
    }

    fn description(&self) -> &'static str {
        "Tests how many keys can be pressed simultaneously"
    }

    fn process_event(&mut self, event: &KeyEvent) {
        if self.start_time.is_none() {
            self.start_time = Some(Instant::now());
        }

        self.total_events += 1;

        match event.event_type {
            KeyEventType::Press => {
                self.pressed_keys.insert(event.key);

                // Check for ghosting before updating max
                self.check_ghosting(event.key, event.timestamp);

                let count = self.pressed_keys.len();
                self.rollover_history.push(count);

                if count > self.max_simultaneous {
                    self.max_simultaneous = count;
                }
            }
            KeyEventType::Release => {
                self.pressed_keys.remove(&event.key);
            }
        }
    }

    fn is_complete(&self) -> bool {
        // This test runs continuously
        false
    }

    fn get_results(&self) -> Vec<TestResult> {
        let mut results = Vec::new();

        // Current state
        results.push(TestResult::info(
            "Currently Pressed",
            format!("{} keys", self.current_count()),
        ));

        // Max rollover
        let status = match self.max_simultaneous {
            0 => ResultStatus::Info,
            1..=2 => ResultStatus::Error,
            3..=5 => ResultStatus::Warning,
            _ => ResultStatus::Ok,
        };
        results.push(TestResult::new(
            "Max Rollover",
            self.rollover_rating(),
            status,
        ));

        results.push(TestResult::info(
            "Peak Keys",
            format!("{} simultaneous", self.max_simultaneous),
        ));

        // Average rollover
        results.push(TestResult::info(
            "Avg Rollover",
            format!("{:.1} keys", self.avg_rollover()),
        ));

        // Ghost detections
        if self.ghost_detections.is_empty() {
            results.push(TestResult::ok("Ghosting", "None detected"));
        } else {
            results.push(TestResult::error(
                "Ghost Events",
                format!("{} detected", self.ghost_detections.len()),
            ));
        }

        // Currently pressed key names
        if !self.pressed_keys.is_empty() {
            let key_names: Vec<String> = self.pressed_keys
                .iter()
                .map(|k| keymap::get_key_info(*k).label.to_string())
                .collect();
            results.push(TestResult::info(
                "Active Keys",
                key_names.join(" + "),
            ));
        }

        results.push(TestResult::info(
            "Total Events",
            format!("{}", self.total_events),
        ));

        results
    }

    fn reset(&mut self) {
        self.pressed_keys.clear();
        self.max_simultaneous = 0;
        self.rollover_history.clear();
        self.ghost_detections.clear();
        self.total_events = 0;
        self.start_time = None;
        self.expected_keys.clear();
    }
}
