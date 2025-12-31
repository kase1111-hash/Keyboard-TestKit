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
#[allow(dead_code)] // Fields stored for future detailed reporting
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

        if self.pressed_keys.len() >= 3
            && !self.expected_keys.is_empty()
            && !self.expected_keys.contains(&new_key)
        {
            self.ghost_detections.push(GhostEvent {
                ghost_key: new_key,
                pressed_keys: self.pressed_keys.iter().copied().collect(),
                timestamp,
            });
            return true;
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

        // Tooltip: Explain what this test measures
        results.push(TestResult::info(
            "--- What This Measures ---",
            "",
        ));
        results.push(TestResult::info(
            "N-Key Rollover = max keys",
            "pressed simultaneously",
        ));
        results.push(TestResult::info(
            "Ghosting = false key press",
            "from matrix limitations",
        ));
        results.push(TestResult::info(
            "Look for: 6KRO+ gaming,",
            "NKRO for pro, no ghosts",
        ));
        results.push(TestResult::info("", ""));

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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_press_event(key: KeyCode) -> KeyEvent {
        KeyEvent {
            key,
            event_type: KeyEventType::Press,
            timestamp: Instant::now(),
            delta_us: 1000,
        }
    }

    fn make_release_event(key: KeyCode) -> KeyEvent {
        KeyEvent {
            key,
            event_type: KeyEventType::Release,
            timestamp: Instant::now(),
            delta_us: 1000,
        }
    }

    #[test]
    fn new_test_initial_state() {
        let test = RolloverTest::new();
        assert_eq!(test.current_count(), 0);
        assert_eq!(test.max_rollover(), 0);
        assert_eq!(test.avg_rollover(), 0.0);
        assert!(test.pressed_keys().is_empty());
    }

    #[test]
    fn rollover_rating_not_tested() {
        let test = RolloverTest::new();
        assert_eq!(test.rollover_rating(), "Not tested");
    }

    #[test]
    fn rollover_rating_2kro() {
        let mut test = RolloverTest::new();
        test.max_simultaneous = 2;
        assert_eq!(test.rollover_rating(), "2KRO");
    }

    #[test]
    fn rollover_rating_6kro() {
        let mut test = RolloverTest::new();
        test.max_simultaneous = 6;
        assert_eq!(test.rollover_rating(), "6KRO");
    }

    #[test]
    fn rollover_rating_nkro() {
        let mut test = RolloverTest::new();
        test.max_simultaneous = 10;
        assert_eq!(test.rollover_rating(), "NKRO");

        test.max_simultaneous = 15;
        assert_eq!(test.rollover_rating(), "NKRO");
    }

    #[test]
    fn process_press_increments_count() {
        let mut test = RolloverTest::new();

        test.process_event(&make_press_event(KeyCode(30))); // A
        assert_eq!(test.current_count(), 1);

        test.process_event(&make_press_event(KeyCode(31))); // S
        assert_eq!(test.current_count(), 2);

        test.process_event(&make_press_event(KeyCode(32))); // D
        assert_eq!(test.current_count(), 3);
    }

    #[test]
    fn process_release_decrements_count() {
        let mut test = RolloverTest::new();

        test.process_event(&make_press_event(KeyCode(30)));
        test.process_event(&make_press_event(KeyCode(31)));
        assert_eq!(test.current_count(), 2);

        test.process_event(&make_release_event(KeyCode(30)));
        assert_eq!(test.current_count(), 1);
    }

    #[test]
    fn max_rollover_tracking() {
        let mut test = RolloverTest::new();

        // Press 4 keys
        for i in 30..34 {
            test.process_event(&make_press_event(KeyCode(i)));
        }
        assert_eq!(test.max_rollover(), 4);

        // Release 2 keys
        test.process_event(&make_release_event(KeyCode(30)));
        test.process_event(&make_release_event(KeyCode(31)));
        assert_eq!(test.current_count(), 2);
        assert_eq!(test.max_rollover(), 4); // Max stays at 4
    }

    #[test]
    fn avg_rollover_calculation() {
        let mut test = RolloverTest::new();

        // Press 3 keys one by one: history = [1, 2, 3]
        test.process_event(&make_press_event(KeyCode(30)));
        test.process_event(&make_press_event(KeyCode(31)));
        test.process_event(&make_press_event(KeyCode(32)));

        // Average of [1, 2, 3] = 2.0
        assert!((test.avg_rollover() - 2.0).abs() < 0.01);
    }

    #[test]
    fn ghost_detection_with_expected_keys() {
        let mut test = RolloverTest::new();

        // Set up expected keys (A, S, D)
        test.expected_keys.insert(KeyCode(30));
        test.expected_keys.insert(KeyCode(31));
        test.expected_keys.insert(KeyCode(32));

        // Press expected keys
        test.process_event(&make_press_event(KeyCode(30)));
        test.process_event(&make_press_event(KeyCode(31)));
        test.process_event(&make_press_event(KeyCode(32)));

        // Press an unexpected key (F = 33) - should trigger ghost detection
        test.process_event(&make_press_event(KeyCode(33)));

        assert_eq!(test.ghost_detections.len(), 1);
    }

    #[test]
    fn no_ghost_detection_without_expected_keys() {
        let mut test = RolloverTest::new();

        // Press 4 keys without setting expected keys
        for i in 30..34 {
            test.process_event(&make_press_event(KeyCode(i)));
        }

        // No ghost detection because expected_keys is empty
        assert!(test.ghost_detections.is_empty());
    }

    #[test]
    fn reset_clears_all() {
        let mut test = RolloverTest::new();

        test.process_event(&make_press_event(KeyCode(30)));
        test.process_event(&make_press_event(KeyCode(31)));
        test.max_simultaneous = 5;
        test.expected_keys.insert(KeyCode(30));

        test.reset();

        assert_eq!(test.current_count(), 0);
        assert_eq!(test.max_rollover(), 0);
        assert!(test.rollover_history.is_empty());
        assert!(test.ghost_detections.is_empty());
        assert_eq!(test.total_events, 0);
        assert!(test.expected_keys.is_empty());
    }

    #[test]
    fn test_name_and_description() {
        let test = RolloverTest::new();
        assert_eq!(test.name(), "N-Key Rollover Test");
        assert!(!test.description().is_empty());
    }

    #[test]
    fn is_never_complete() {
        let mut test = RolloverTest::new();
        test.process_event(&make_press_event(KeyCode(30)));
        assert!(!test.is_complete()); // Continuous test
    }

    #[test]
    fn total_events_counted() {
        let mut test = RolloverTest::new();

        test.process_event(&make_press_event(KeyCode(30)));
        test.process_event(&make_release_event(KeyCode(30)));
        test.process_event(&make_press_event(KeyCode(31)));

        assert_eq!(test.total_events, 3);
    }
}
