//! Stickiness detection test module

use super::{KeyboardTest, TestResult};
use crate::keyboard::{keymap, KeyCode, KeyEvent, KeyEventType};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Record of a key that was held past the threshold
#[derive(Debug, Clone)]
struct StickyKeyRecord {
    key: KeyCode,
    duration_at_detection: Duration,
    occurrences: u32,
    /// How many of those holds ended in a normal release
    released: u32,
    /// Longest of those released holds
    longest_released: Duration,
}

/// Test for detecting stuck or sticky keys.
///
/// A key is *stuck* only while it is still down past the threshold. Once it
/// comes back up it was a long hold, not a fault: it stays in the history
/// (`flagged_keys`, `long_holds`) but is no longer reported as stuck.
pub struct StickinessTest {
    /// Threshold duration after which a held key is flagged
    threshold: Duration,
    /// Currently held keys with their press times
    held_keys: HashMap<KeyCode, Instant>,
    /// Every key that has been held past the threshold (history, including
    /// holds that have since been released)
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

    /// Check all held keys for stickiness. Returns the keys that crossed the
    /// threshold since the last check.
    pub fn check_stuck_keys(&mut self) -> Vec<KeyCode> {
        self.check_stuck_keys_at(Instant::now())
    }

    fn check_stuck_keys_at(&mut self, now: Instant) -> Vec<KeyCode> {
        let mut newly_stuck = Vec::new();

        for (&key, &press_time) in &self.held_keys {
            let duration = now.saturating_duration_since(press_time);

            if duration > self.threshold && !self.currently_flagged.contains_key(&key) {
                self.currently_flagged.insert(key, now);

                // Check if we already have a record for this key
                if let Some(record) = self.flagged_keys.iter_mut().find(|r| r.key == key) {
                    record.occurrences += 1;
                } else {
                    self.flagged_keys.push(StickyKeyRecord {
                        key,
                        duration_at_detection: duration,
                        occurrences: 1,
                        released: 0,
                        longest_released: Duration::ZERO,
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
            .map(|(&key, &press_time)| (key, now.saturating_duration_since(press_time)))
            .collect()
    }

    /// Keys that have ever been held past the threshold, including holds that
    /// were released since: (key, hold duration at detection, occurrences).
    /// Use `stuck_keys` for keys that are stuck right now.
    pub fn flagged_keys(&self) -> Vec<(KeyCode, Duration, u32)> {
        self.flagged_keys
            .iter()
            .map(|r| (r.key, r.duration_at_detection, r.occurrences))
            .collect()
    }

    /// Keys that are stuck right now: still down past the threshold, with how
    /// long they have been down. Longest first.
    pub fn stuck_keys(&self) -> Vec<(KeyCode, Duration)> {
        self.stuck_keys_at(Instant::now())
    }

    fn stuck_keys_at(&self, now: Instant) -> Vec<(KeyCode, Duration)> {
        let mut stuck: Vec<(KeyCode, Duration)> = self
            .held_keys
            .iter()
            .map(|(&key, &press_time)| (key, now.saturating_duration_since(press_time)))
            .filter(|(_, held)| *held > self.threshold)
            .collect();
        stuck.sort_by(|a, b| b.1.cmp(&a.1).then(a.0 .0.cmp(&b.0 .0)));
        stuck
    }

    /// Keys that were held past the threshold and then released normally:
    /// (key, longest such hold, how many times). These are long holds, not
    /// faults, unless nobody was holding the key.
    pub fn long_holds(&self) -> Vec<(KeyCode, Duration, u32)> {
        self.flagged_keys
            .iter()
            .filter(|r| r.released > 0)
            .map(|r| (r.key, r.longest_released, r.released))
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
                let pressed_at = self.held_keys.remove(&event.key);
                // A flagged key that comes back up was a long hold, not a
                // stuck key: move it to the long-hold history.
                if self.currently_flagged.remove(&event.key).is_some() {
                    if let Some(record) = self.flagged_keys.iter_mut().find(|r| r.key == event.key)
                    {
                        let held = pressed_at
                            .map(|t| event.timestamp.saturating_duration_since(t))
                            .unwrap_or(record.duration_at_detection);
                        record.released += 1;
                        record.longest_released = record.longest_released.max(held);
                    }
                }
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
            "Detects keys that stay",
            "'pressed' after release",
        ));
        results.push(TestResult::info(
            "Causes: debris, worn switch",
            "or driver/software issue",
        ));
        results.push(TestResult::info(
            "Look for: no stuck keys,",
            "clean release behavior",
        ));
        results.push(TestResult::info("", ""));

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

        // Keys stuck right now: the actual fault.
        let stuck = self.stuck_keys();
        if stuck.is_empty() {
            results.push(TestResult::ok("Stuck Keys", "None detected"));
        } else {
            results.push(TestResult::error(
                "Stuck Keys",
                format!("{} still down", stuck.len()),
            ));
            for (key, held) in &stuck {
                let key_info = keymap::get_key_info(*key);
                results.push(TestResult::error(
                    format!("  {}", key_info.name),
                    format!("down {:.1}s", held.as_secs_f64()),
                ));
            }
            results.push(TestResult::info("  Not holding it?", "the switch is stuck"));
        }

        // Past long holds that ended in a normal release. Only a fault if the
        // key was not being held on purpose (then it is sticking).
        let long_holds = self.long_holds();
        if !long_holds.is_empty() {
            results.push(TestResult::info(
                "Long Holds",
                format!("{} key(s), released normally", long_holds.len()),
            ));
            for (key, longest, count) in &long_holds {
                let key_info = keymap::get_key_info(*key);
                results.push(TestResult::info(
                    format!("  {}", key_info.name),
                    format!("{}x, longest {:.1}s", count, longest.as_secs_f64()),
                ));
            }
        }

        // Keys that are down but have not reached the threshold yet
        let mut held: Vec<(KeyCode, Duration)> = self
            .held_keys()
            .into_iter()
            .filter(|(_, d)| *d <= self.threshold)
            .collect();
        held.sort_by_key(|(key, _)| key.0);
        if !held.is_empty() {
            results.push(TestResult::info("--- Held Keys ---", ""));
            for (key, duration) in held {
                let key_info = keymap::get_key_info(key);
                results.push(TestResult::info(
                    format!("  {}", key_info.name),
                    format!("{:.1}s", duration.as_secs_f64()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::ResultStatus;

    const BACKSPACE: KeyCode = KeyCode(14);

    fn evt(key: KeyCode, kind: KeyEventType, at: Instant) -> KeyEvent {
        KeyEvent::new(key, kind, at, 0)
    }

    /// An instant `ago` in the past (falls back to now on a freshly booted clock).
    fn ago(ago: Duration) -> Instant {
        Instant::now().checked_sub(ago).unwrap_or_else(Instant::now)
    }

    fn faults(test: &StickinessTest) -> Vec<TestResult> {
        test.get_results()
            .into_iter()
            .filter(|r| matches!(r.status, ResultStatus::Error | ResultStatus::Warning))
            .collect()
    }

    #[test]
    fn normal_typing_is_not_flagged() {
        // Imperfect typing: keystrokes of 60-400 ms, overlapping keys
        let mut test = StickinessTest::new(2000);
        let t0 = Instant::now();
        for i in 0..40u64 {
            let key = KeyCode(16 + (i % 10) as u16);
            let down = t0 + Duration::from_millis(i * 120);
            let up = down + Duration::from_millis(60 + (i % 5) * 85);
            test.process_event(&evt(key, KeyEventType::Press, down));
            test.process_event(&evt(key, KeyEventType::Release, up));
            assert!(test.check_stuck_keys_at(up).is_empty());
        }
        assert!(test.stuck_keys_at(t0 + Duration::from_secs(10)).is_empty());
        assert!(test.flagged_keys().is_empty());
        assert!(test.long_holds().is_empty());
        assert!(faults(&test).is_empty());
    }

    #[test]
    fn key_that_stays_down_is_stuck() {
        let mut test = StickinessTest::new(2000);
        let t0 = Instant::now();
        test.process_event(&evt(BACKSPACE, KeyEventType::Press, t0));
        assert!(test
            .check_stuck_keys_at(t0 + Duration::from_millis(1900))
            .is_empty());
        let now = t0 + Duration::from_secs(3);
        assert_eq!(test.check_stuck_keys_at(now), vec![BACKSPACE]);
        // Reported once, not on every check
        assert!(test.check_stuck_keys_at(now).is_empty());
        assert_eq!(
            test.stuck_keys_at(now),
            vec![(BACKSPACE, Duration::from_secs(3))]
        );
        assert_eq!(test.flagged_keys().len(), 1);
        assert!(test.long_holds().is_empty());
    }

    #[test]
    fn stuck_key_is_shown_as_fault() {
        let mut test = StickinessTest::new(2000);
        test.process_event(&evt(
            BACKSPACE,
            KeyEventType::Press,
            ago(Duration::from_secs(3)),
        ));
        test.check_stuck_keys();
        let faults = faults(&test);
        assert!(faults
            .iter()
            .any(|r| r.label == "Stuck Keys" && r.status == ResultStatus::Error));
        assert!(faults.iter().any(|r| r.label.contains("Backspace")));
    }

    #[test]
    fn released_long_hold_is_not_stuck() {
        // Holding Backspace to delete a line, then letting go, is not a fault
        let mut test = StickinessTest::new(2000);
        let t0 = Instant::now();
        test.process_event(&evt(BACKSPACE, KeyEventType::Press, t0));
        test.check_stuck_keys_at(t0 + Duration::from_millis(2100));
        let up = t0 + Duration::from_millis(3200);
        test.process_event(&evt(BACKSPACE, KeyEventType::Release, up));

        assert!(test.stuck_keys_at(up + Duration::from_secs(5)).is_empty());
        assert_eq!(
            test.long_holds(),
            vec![(BACKSPACE, Duration::from_millis(3200), 1)]
        );
        // History is kept for the analysis
        assert_eq!(test.flagged_keys().len(), 1);
    }

    #[test]
    fn released_long_hold_is_listed_as_info_only() {
        let mut test = StickinessTest::new(2000);
        test.process_event(&evt(
            BACKSPACE,
            KeyEventType::Press,
            ago(Duration::from_secs(3)),
        ));
        test.check_stuck_keys();
        test.process_event(&evt(BACKSPACE, KeyEventType::Release, Instant::now()));

        assert!(faults(&test).is_empty(), "{:?}", faults(&test));
        let results = test.get_results();
        assert!(results
            .iter()
            .any(|r| r.label == "Stuck Keys" && r.status == ResultStatus::Ok));
        assert!(results
            .iter()
            .any(|r| r.label == "Long Holds" && r.status == ResultStatus::Info));
    }

    #[test]
    fn key_stuck_again_after_a_long_hold_is_stuck() {
        let mut test = StickinessTest::new(2000);
        let t0 = Instant::now();
        test.process_event(&evt(BACKSPACE, KeyEventType::Press, t0));
        test.check_stuck_keys_at(t0 + Duration::from_secs(3));
        test.process_event(&evt(
            BACKSPACE,
            KeyEventType::Release,
            t0 + Duration::from_secs(4),
        ));
        let t1 = t0 + Duration::from_secs(10);
        test.process_event(&evt(BACKSPACE, KeyEventType::Press, t1));
        let now = t1 + Duration::from_secs(5);
        assert_eq!(test.check_stuck_keys_at(now), vec![BACKSPACE]);
        assert_eq!(test.stuck_keys_at(now).len(), 1);
        assert_eq!(
            test.flagged_keys(),
            vec![(BACKSPACE, Duration::from_secs(3), 2)]
        );
        assert_eq!(
            test.long_holds(),
            vec![(BACKSPACE, Duration::from_secs(4), 1)]
        );
    }

    #[test]
    fn raised_threshold_unflags_a_held_key() {
        let mut test = StickinessTest::new(2000);
        let t0 = Instant::now();
        test.process_event(&evt(BACKSPACE, KeyEventType::Press, t0));
        let now = t0 + Duration::from_secs(3);
        assert_eq!(test.stuck_keys_at(now).len(), 1);
        test.set_threshold(15_000);
        assert!(test.stuck_keys_at(now).is_empty());
        assert!(test.check_stuck_keys_at(now).is_empty());
    }

    #[test]
    fn reset_clears_history() {
        let mut test = StickinessTest::new(2000);
        let t0 = Instant::now();
        test.process_event(&evt(BACKSPACE, KeyEventType::Press, t0));
        test.check_stuck_keys_at(t0 + Duration::from_secs(3));
        test.reset();
        assert!(test.flagged_keys().is_empty());
        assert!(test.held_keys().is_empty());
        assert!(test.stuck_keys().is_empty());
    }
}
