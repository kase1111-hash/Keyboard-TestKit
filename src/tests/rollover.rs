//! N-Key Rollover and Ghosting test module

use super::auto_test::GOOD_ROLLOVER;
use super::{InputSource, KeyboardTest, ResultStatus, TestResult};
use crate::keyboard::{keymap, KeyCode, KeyEvent, KeyEventType};
use std::collections::HashSet;
use std::time::Instant;

/// Test for N-Key Rollover: how many keys register at once.
///
/// Ghost keys are shown (Active Keys) but not detected automatically: a ghost
/// arrives as an ordinary press (see [`RolloverTest::ghost_count`]).
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
    /// Whether key releases are real. With press-only terminal input releases
    /// are synthesized on a timer, so the number of keys "down" at once is
    /// a guess.
    real_releases: bool,
}

/// A detected ghosting event
#[derive(Debug, Clone)]
struct GhostEvent {
    ghost_key: KeyCode,
    context_keys: usize,
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
            real_releases: true,
        }
    }

    /// Tell the test where events come from. Without real releases the peak
    /// count is not a measurement and is not shown.
    pub fn set_input_source(&mut self, source: InputSource) {
        self.real_releases = source.has_release_events();
    }

    /// Whether the peak rollover is a measurement with the current input
    /// source (key releases are real).
    pub fn rollover_measurable(&self) -> bool {
        self.real_releases
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

    /// Number of ghost key events detected.
    ///
    /// Ghosting is not detected automatically: a ghost key arrives as an
    /// ordinary press, and telling it apart needs to know which keys the user
    /// is holding (`expected_keys`), which nothing sets yet. This is 0 unless
    /// that information is supplied.
    pub fn ghost_count(&self) -> usize {
        self.ghost_detections.len()
    }

    /// Get currently pressed keys
    pub fn pressed_keys(&self) -> Vec<KeyCode> {
        self.pressed_keys.iter().copied().collect()
    }

    /// Check for potential ghosting
    /// Returns true if ghosting was detected
    // TODO: Real ghosting detection needs keyboard matrix layout knowledge.
    // This heuristic only works when expected_keys is pre-populated.
    fn check_ghosting(&mut self, new_key: KeyCode) -> bool {
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
                context_keys: self.pressed_keys.len(),
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
                self.check_ghosting(event.key);

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
        results.push(TestResult::info("--- What This Measures ---", ""));
        results.push(TestResult::info(
            "N-Key Rollover = max keys",
            "pressed simultaneously",
        ));
        results.push(TestResult::info(
            "Ghosting = extra key that",
            "you did not press shows up",
        ));
        results.push(TestResult::info(
            "Look for: 6KRO+ gaming,",
            "NKRO for pro, no extra keys",
        ));
        results.push(TestResult::info("", ""));

        // Current state
        results.push(TestResult::info(
            "Currently Pressed",
            format!("{} keys", self.current_count()),
        ));

        // Max rollover
        if !self.real_releases {
            // Releases are guessed: a tap stays "down" for up to 750 ms, so
            // fast typing looks like several keys held at once, while the
            // keys of a held chord are let go once autorepeat moves on to
            // the last one. The peak is neither an upper nor a lower bound.
            results.push(TestResult::info("Max Rollover", "n/a (terminal input)"));
            results.push(TestResult::info(
                "Terminal reports",
                "presses only; releases guessed",
            ));
            results.push(TestResult::info(
                "rollover needs",
                "releases: evdev or kitty protocol",
            ));
        } else {
            // 1-2 keys is not enough evidence either way (hold more keys to
            // test). 3-5 may be the keyboard's limit or just the number of
            // keys the user held, which the test cannot tell apart.
            let status = match self.max_simultaneous {
                0..=2 => ResultStatus::Info,
                n if n < GOOD_ROLLOVER => ResultStatus::Warning,
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
            if status == ResultStatus::Warning {
                results.push(TestResult::info(
                    format!("  Held more than {}?", self.max_simultaneous),
                    "the keyboard limits rollover",
                ));
                results.push(TestResult::info(
                    format!("  Held only {}?", self.max_simultaneous),
                    format!("not a fault: hold {}+ keys", GOOD_ROLLOVER),
                ));
            }
            results.push(TestResult::info(
                "Avg Rollover",
                format!("{:.1} keys", self.avg_rollover()),
            ));
        }

        // Ghost detections. A ghost arrives as an ordinary key press, so it
        // cannot be told apart without knowing which keys are really held.
        if self.ghost_detections.is_empty() {
            results.push(TestResult::info(
                "Ghosting",
                "not auto-detected: watch Active Keys for extras",
            ));
        } else {
            results.push(TestResult::error(
                "Ghost Events",
                format!("{} detected", self.ghost_detections.len()),
            ));
            for ghost in self.ghost_detections.iter().take(3) {
                let key_info = keymap::get_key_info(ghost.ghost_key);
                results.push(TestResult::error(
                    format!("  Ghost: {}", key_info.label),
                    format!("with {} keys held", ghost.context_keys),
                ));
            }
        }

        // Currently pressed key names
        if !self.pressed_keys.is_empty() {
            let key_names: Vec<String> = self
                .pressed_keys
                .iter()
                .map(|k| keymap::get_key_info(*k).label.to_string())
                .collect();
            results.push(TestResult::info("Active Keys", key_names.join(" + ")));
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
    use crate::tests::test_helpers::{press, release};

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

        test.process_event(&press(KeyCode(30))); // A
        assert_eq!(test.current_count(), 1);

        test.process_event(&press(KeyCode(31))); // S
        assert_eq!(test.current_count(), 2);

        test.process_event(&press(KeyCode(32))); // D
        assert_eq!(test.current_count(), 3);
    }

    #[test]
    fn process_release_decrements_count() {
        let mut test = RolloverTest::new();

        test.process_event(&press(KeyCode(30)));
        test.process_event(&press(KeyCode(31)));
        assert_eq!(test.current_count(), 2);

        test.process_event(&release(KeyCode(30)));
        assert_eq!(test.current_count(), 1);
    }

    #[test]
    fn max_rollover_tracking() {
        let mut test = RolloverTest::new();

        // Press 4 keys
        for i in 30..34 {
            test.process_event(&press(KeyCode(i)));
        }
        assert_eq!(test.max_rollover(), 4);

        // Release 2 keys
        test.process_event(&release(KeyCode(30)));
        test.process_event(&release(KeyCode(31)));
        assert_eq!(test.current_count(), 2);
        assert_eq!(test.max_rollover(), 4); // Max stays at 4
    }

    #[test]
    fn avg_rollover_calculation() {
        let mut test = RolloverTest::new();

        // Press 3 keys one by one: history = [1, 2, 3]
        test.process_event(&press(KeyCode(30)));
        test.process_event(&press(KeyCode(31)));
        test.process_event(&press(KeyCode(32)));

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
        test.process_event(&press(KeyCode(30)));
        test.process_event(&press(KeyCode(31)));
        test.process_event(&press(KeyCode(32)));

        // Press an unexpected key (F = 33) - should trigger ghost detection
        test.process_event(&press(KeyCode(33)));

        assert_eq!(test.ghost_detections.len(), 1);
    }

    #[test]
    fn no_ghost_detection_without_expected_keys() {
        let mut test = RolloverTest::new();

        // Press 4 keys without setting expected keys
        for i in 30..34 {
            test.process_event(&press(KeyCode(i)));
        }

        // No ghost detection because expected_keys is empty
        assert!(test.ghost_detections.is_empty());
    }

    #[test]
    fn reset_clears_all() {
        let mut test = RolloverTest::new();

        test.process_event(&press(KeyCode(30)));
        test.process_event(&press(KeyCode(31)));
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
        test.process_event(&press(KeyCode(30)));
        assert!(!test.is_complete()); // Continuous test
    }

    #[test]
    fn ghosting_is_not_claimed_clean_when_it_was_not_measured() {
        // A membrane keyboard that ghosts: holding A, S, D makes F appear.
        // The test cannot tell F from a real press, so it must not say
        // "None detected" in green.
        let mut test = RolloverTest::new();
        for key in [30, 31, 32, 33] {
            test.process_event(&press(KeyCode(key)));
        }
        let results = test.get_results();
        let ghost = results
            .iter()
            .find(|r| r.label == "Ghosting")
            .expect("ghosting row");
        assert_eq!(ghost.status, ResultStatus::Info);
        assert!(!ghost.value.contains("None"), "{:?}", ghost);
        // ...but the extra key is visible for the user to judge
        let active = results.iter().find(|r| r.label == "Active Keys").unwrap();
        assert!(active.value.contains('F'), "{:?}", active);
    }

    fn max_rollover_status(test: &RolloverTest) -> ResultStatus {
        test.get_results()
            .into_iter()
            .find(|r| r.label == "Max Rollover")
            .unwrap()
            .status
    }

    #[test]
    fn limited_rollover_is_flagged_with_real_releases() {
        // A 4KRO keyboard: only 4 of the keys the user holds register
        let mut test = RolloverTest::new();
        test.set_input_source(InputSource::Evdev);
        for key in 30..34 {
            test.process_event(&press(KeyCode(key)));
        }
        assert_eq!(max_rollover_status(&test), ResultStatus::Warning);
        // A keyboard that registers 7 held keys is fine
        for key in 34..37 {
            test.process_event(&press(KeyCode(key)));
        }
        assert_eq!(max_rollover_status(&test), ResultStatus::Ok);
        assert!(!test
            .get_results()
            .iter()
            .any(|r| r.label.contains("Held only")));
    }

    #[test]
    fn low_peak_is_worded_as_a_condition() {
        // 4 keys at once: a 4KRO keyboard, or a user who held only 4 keys on
        // a healthy one. The view must not call it the keyboard's limit.
        let mut test = RolloverTest::new();
        test.set_input_source(InputSource::Evdev);
        for key in 30..34 {
            test.process_event(&press(KeyCode(key)));
        }
        let results = test.get_results();
        let more = results
            .iter()
            .find(|r| r.label.contains("Held more than 4?"))
            .expect("condition for a real limit");
        assert!(more.value.contains("limits rollover"));
        let only = results
            .iter()
            .find(|r| r.label.contains("Held only 4?"))
            .expect("condition for a user who held few keys");
        assert!(only.value.contains("not a fault"), "{:?}", only);
    }

    #[test]
    fn press_only_input_does_not_show_a_rollover_number() {
        // A press-only terminal keeps a tap "down" for up to 750 ms: typing
        // five letters quickly looks like five keys held at once, although
        // the user never held more than one. No count, no grade.
        let mut test = RolloverTest::new();
        test.set_input_source(InputSource::TerminalPressOnly);
        for key in 30..35 {
            test.process_event(&press(KeyCode(key)));
        }
        assert_eq!(test.max_rollover(), 5);
        assert!(!test.rollover_measurable());
        let results = test.get_results();
        let max = results.iter().find(|r| r.label == "Max Rollover").unwrap();
        assert_eq!(max.status, ResultStatus::Info);
        assert_eq!(max.value, "n/a (terminal input)");
        assert!(!results
            .iter()
            .any(|r| r.value.contains("5KRO") || r.value.contains("5 simultaneous")));
    }

    #[test]
    fn total_events_counted() {
        let mut test = RolloverTest::new();

        test.process_event(&press(KeyCode(30)));
        test.process_event(&release(KeyCode(30)));
        test.process_event(&press(KeyCode(31)));

        assert_eq!(test.total_events, 3);
    }
}
