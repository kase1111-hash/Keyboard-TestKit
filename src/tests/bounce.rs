//! Hold down and release test with bounce detection

use super::{InputSource, KeyboardTest, TestResult};
use crate::keyboard::{keymap, KeyCode, KeyEvent, KeyEventType};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

/// Default re-press (release -> press) bounce window in milliseconds.
///
/// To press a key again the finger has to lift it and come back down; even
/// fast same-key drumming leaves 30 ms or more between the release and the
/// next press. A worn or dirty switch that chatters re-registers within a
/// report or two of the release (8-16 ms on a 125 Hz keyboard, 5-15 ms past
/// typical ~5 ms firmware debounce on a 1000 Hz one).
pub const DEFAULT_BOUNCE_WINDOW_MS: u64 = 20;

/// Smallest accepted re-press window. A shorter window misses chatter that a
/// 125 Hz keyboard can only report 8 or 16 ms after the release, and no finger
/// re-presses that fast anyway, so shorter settings (including the 5 ms
/// default of older config files) are raised to this.
pub const MIN_BOUNCE_WINDOW_MS: u64 = 20;

/// Largest accepted re-press window. Fast same-key drumming brings the
/// release -> press gap down to ~30-40 ms, so a wider window would flag a
/// healthy key under a quick typist.
pub const MAX_BOUNCE_WINDOW_MS: u64 = 40;

/// Two presses of the same key closer than this are chatter whatever the
/// release in between looked like: 40 ms is a 25 Hz tap rate, while the
/// fastest same-key tapping is ~10-15 Hz (65 ms or more per tap).
pub const CHATTER_PRESS_GAP_MS: u64 = 40;

/// Whether a press re-registers the same key too soon to be a new keystroke.
///
/// `last_press` and `last_release` are the previous press and release of the
/// same key. Only meaningful with device (kernel) timestamps: terminal input
/// is stamped when it is read, so a busy frame can squeeze two real presses
/// together.
pub fn is_chatter_press(
    at: Instant,
    last_press: Option<Instant>,
    last_release: Option<Instant>,
    release_window: Duration,
) -> bool {
    let within = |prev: Option<Instant>, window: Duration| {
        prev.is_some_and(|p| at.saturating_duration_since(p) < window)
    };
    within(last_release, release_window)
        || within(last_press, Duration::from_millis(CHATTER_PRESS_GAP_MS))
}

/// Statistics for a single key's hold/release behavior
#[derive(Debug, Clone, Default)]
struct KeyHoldStats {
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
    /// Previous press of this key (for chatter detection)
    last_press: Option<Instant>,
    /// Previous release of this key (for chatter detection)
    last_release: Option<Instant>,
}

/// Test for hold down, release, and bounce detection
pub struct HoldReleaseTest {
    /// Per-key statistics
    key_stats: HashMap<KeyCode, KeyHoldStats>,
    /// Re-press bounce window: a press this soon after the key's release is a bounce
    bounce_window: Duration,
    /// Whether event timestamps come from the device (needed for bounce checks)
    precise_timestamps: bool,
    /// Whether releases are real (hold times are meaningless when synthesized)
    real_releases: bool,
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
    repeat_intervals: VecDeque<u64>,
}

impl HoldReleaseTest {
    /// Create the test with a re-press bounce window in milliseconds (clamped
    /// to [`MIN_BOUNCE_WINDOW_MS`]..=[`MAX_BOUNCE_WINDOW_MS`]).
    ///
    /// Timestamps are assumed to be precise and releases real until
    /// [`HoldReleaseTest::set_input_source`] says otherwise.
    pub fn new(bounce_window_ms: u64) -> Self {
        Self {
            key_stats: HashMap::new(),
            bounce_window: Self::clamp_window(bounce_window_ms),
            precise_timestamps: true,
            real_releases: true,
            total_bounces: 0,
            total_presses: 0,
            held_keys: Vec::new(),
            start_time: None,
            last_event_time: None,
            repeat_intervals: VecDeque::new(),
        }
    }

    fn clamp_window(ms: u64) -> Duration {
        Duration::from_millis(ms.clamp(MIN_BOUNCE_WINDOW_MS, MAX_BOUNCE_WINDOW_MS))
    }

    /// Tell the test where events come from. Bounce detection needs device
    /// timestamps; hold times need real releases.
    pub fn set_input_source(&mut self, source: InputSource) {
        self.precise_timestamps = source.has_precise_timestamps();
        self.real_releases = source.has_release_events();
    }

    /// Change the re-press bounce window (clamped like [`HoldReleaseTest::new`]).
    pub fn set_bounce_window_ms(&mut self, bounce_window_ms: u64) {
        self.bounce_window = Self::clamp_window(bounce_window_ms);
    }

    /// The re-press bounce window in effect, in milliseconds.
    pub fn bounce_window_ms(&self) -> u64 {
        self.bounce_window.as_millis() as u64
    }

    /// Whether bounces can be measured with the current input source.
    pub fn bounce_measurable(&self) -> bool {
        self.precise_timestamps
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
        let avg_interval_us =
            self.repeat_intervals.iter().sum::<u64>() as f64 / self.repeat_intervals.len() as f64;
        if avg_interval_us > 0.0 {
            Some(1_000_000.0 / avg_interval_us)
        } else {
            None
        }
    }

    /// Total bounces detected across all keys
    pub fn total_bounces(&self) -> u32 {
        self.total_bounces
    }

    /// Total key presses seen
    pub fn total_presses(&self) -> u32 {
        self.total_presses
    }

    /// Shortest completed hold duration per key, in milliseconds.
    pub fn min_hold_by_key(&self) -> Vec<(KeyCode, f64)> {
        self.key_stats
            .iter()
            .filter_map(|(k, s)| s.min_hold_ms.map(|m| (*k, m)))
            .collect()
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
        Self::new(DEFAULT_BOUNCE_WINDOW_MS)
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
        let check_bounce = self.precise_timestamps;
        let stats = self.key_stats.entry(event.key).or_default();

        match event.event_type {
            KeyEventType::Press => {
                // A bounce is the switch registering the same key again too
                // soon after its release/press. Terminal timestamps are read
                // times, so this is only judged with device timestamps.
                if check_bounce
                    && is_chatter_press(
                        event.timestamp,
                        stats.last_press,
                        stats.last_release,
                        bounce_window,
                    )
                {
                    stats.bounce_count += 1;
                    self.total_bounces += 1;
                }
                stats.last_press = Some(event.timestamp);
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
                    if interval < 500_000 {
                        // Only track intervals < 500ms
                        self.repeat_intervals.push_back(interval);
                        if self.repeat_intervals.len() > 100 {
                            self.repeat_intervals.pop_front();
                        }
                    }
                }
                self.last_event_time = Some(event.timestamp);
            }
            KeyEventType::Release => {
                stats.is_pressed = false;
                stats.last_release = Some(event.timestamp);

                // Calculate hold duration
                if let Some(start) = stats.press_start.take() {
                    let hold_ms = event.timestamp.duration_since(start).as_secs_f64() * 1000.0;
                    stats.total_hold_ms += hold_ms;

                    stats.min_hold_ms =
                        Some(stats.min_hold_ms.map(|m| m.min(hold_ms)).unwrap_or(hold_ms));
                    stats.max_hold_ms =
                        Some(stats.max_hold_ms.map(|m| m.max(hold_ms)).unwrap_or(hold_ms));
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
        results.push(TestResult::info("--- What This Measures ---", ""));
        results.push(TestResult::info(
            "Tests key hold/release and",
            "detects switch bounce",
        ));
        results.push(TestResult::info(
            "Bounce = one keystroke that",
            "registers twice (chatter)",
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
        if !self.precise_timestamps {
            results.push(TestResult::info("Bounces Detected", "n/a (terminal input)"));
            results.push(TestResult::info(
                "Terminal timestamps",
                "are read times, too coarse;",
            ));
            results.push(TestResult::info(
                "bounce checks need",
                "evdev (Linux, sudo/input group)",
            ));
        } else {
            if self.total_presses == 0 {
                results.push(TestResult::info("Bounces Detected", "not tested yet"));
            } else if self.total_bounces == 0 {
                results.push(TestResult::ok("Bounces Detected", "None"));
            } else {
                results.push(TestResult::error(
                    "Bounces Detected",
                    format!("{}", self.total_bounces),
                ));
            }
            results.push(TestResult::info(
                "Bounce Window",
                format!(
                    "re-press <{} ms after release or <{} ms after press",
                    self.bounce_window.as_millis(),
                    CHATTER_PRESS_GAP_MS
                ),
            ));
        }

        // Hold duration stats
        if !self.real_releases {
            results.push(TestResult::info(
                "Avg Hold Time",
                "n/a (releases synthesized)",
            ));
        } else if let Some(avg) = self.avg_hold_ms() {
            results.push(TestResult::info("Avg Hold Time", format!("{:.1} ms", avg)));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::test_helpers::{press_at, release_at};
    use crate::tests::ResultStatus;

    const KEY: KeyCode = KeyCode(18); // E

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// Feed (offset_ms, is_press) events for one key.
    fn feed(test: &mut HoldReleaseTest, key: KeyCode, t0: Instant, events: &[(u64, bool)]) {
        for &(at, is_press) in events {
            if is_press {
                test.process_event(&press_at(key, t0 + ms(at), 0));
            } else {
                test.process_event(&release_at(key, t0 + ms(at)));
            }
        }
    }

    fn row<'a>(results: &'a [TestResult], label: &str) -> &'a TestResult {
        results
            .iter()
            .find(|r| r.label == label)
            .unwrap_or_else(|| panic!("no '{}' row in {:?}", label, results))
    }

    #[test]
    fn chatter_shortly_after_release_is_detected() {
        // Worn switch on a 125 Hz keyboard: the re-press shows up two reports
        // (16 ms) after the release. The old 5 ms window could never see it.
        let mut test = HoldReleaseTest::default();
        let t0 = Instant::now();
        feed(
            &mut test,
            KEY,
            t0,
            &[(0, true), (70, false), (86, true), (150, false)],
        );
        assert_eq!(test.total_bounces(), 1);
        assert_eq!(test.bouncy_keys(), vec![(KEY, 1)]);
        assert_eq!(
            row(&test.get_results(), "Bounces Detected").status,
            ResultStatus::Error
        );
    }

    #[test]
    fn chatter_by_short_press_to_press_gap_is_detected() {
        // 15 ms blip, released, and registered again 23 ms later: the release
        // gap alone is over the window but 38 ms press-to-press is no finger.
        let mut test = HoldReleaseTest::default();
        let t0 = Instant::now();
        feed(
            &mut test,
            KEY,
            t0,
            &[(0, true), (15, false), (38, true), (100, false)],
        );
        assert_eq!(test.total_bounces(), 1);
    }

    #[test]
    fn each_chatter_counts_once() {
        // press, 2 ms blip release, re-press: one doubled keystroke, one bounce
        let mut test = HoldReleaseTest::default();
        let t0 = Instant::now();
        feed(
            &mut test,
            KEY,
            t0,
            &[(0, true), (1, false), (2, true), (90, false)],
        );
        assert_eq!(test.total_bounces(), 1);
    }

    #[test]
    fn fast_human_typing_is_not_bounce() {
        // Double letters and fast same-key drumming by a real person: holds
        // of 35-80 ms, release -> press gaps of 30+ ms, press -> press 70+ ms.
        let mut test = HoldReleaseTest::default();
        let t0 = Instant::now();
        feed(
            &mut test,
            KEY,
            t0,
            &[
                (0, true),
                (62, false),
                (110, true), // "ee" in normal typing
                (175, false),
                (400, true), // drumming at ~13 Hz with uneven timing
                (437, false),
                (470, true),
                (512, false),
                (548, true),
                (583, false),
                (621, true),
                (660, false),
            ],
        );
        // Rolled typing of different keys overlapping by a few ms
        let t1 = t0 + ms(1000);
        for (i, key) in [30u16, 31, 32, 33].iter().enumerate() {
            let at = t1 + ms(i as u64 * 4);
            test.process_event(&press_at(KeyCode(*key), at, 0));
            test.process_event(&release_at(KeyCode(*key), at + ms(60)));
        }
        // A light graze: very short press, never re-registered
        feed(
            &mut test,
            KeyCode(34),
            t0 + ms(2000),
            &[(0, true), (8, false)],
        );

        assert_eq!(test.total_bounces(), 0, "{:?}", test.bouncy_keys());
        assert_eq!(
            row(&test.get_results(), "Bounces Detected").status,
            ResultStatus::Ok
        );
    }

    #[test]
    fn old_five_ms_setting_is_raised_to_a_useful_window() {
        let mut test = HoldReleaseTest::new(5);
        assert_eq!(test.bounce_window_ms(), MIN_BOUNCE_WINDOW_MS);
        let t0 = Instant::now();
        feed(&mut test, KEY, t0, &[(0, true), (70, false), (82, true)]);
        assert_eq!(test.total_bounces(), 1);
    }

    #[test]
    fn bounce_window_setter_applies_and_clamps() {
        let mut test = HoldReleaseTest::default();
        assert_eq!(test.bounce_window_ms(), DEFAULT_BOUNCE_WINDOW_MS);

        test.set_bounce_window_ms(30);
        assert_eq!(test.bounce_window_ms(), 30);
        let t0 = Instant::now();
        // 25 ms release -> re-press: inside a 30 ms window, outside the default
        feed(&mut test, KEY, t0, &[(0, true), (60, false), (85, true)]);
        assert_eq!(test.total_bounces(), 1);

        test.set_bounce_window_ms(500);
        assert_eq!(test.bounce_window_ms(), MAX_BOUNCE_WINDOW_MS);
        test.set_bounce_window_ms(0);
        assert_eq!(test.bounce_window_ms(), MIN_BOUNCE_WINDOW_MS);
    }

    #[test]
    fn terminal_timestamps_do_not_judge_bounce() {
        // Two real presses read in the same frame look 1 ms apart on a
        // terminal; that must not become a "bounce".
        let mut test = HoldReleaseTest::default();
        test.set_input_source(InputSource::TerminalWithRelease);
        let t0 = Instant::now();
        feed(
            &mut test,
            KEY,
            t0,
            &[(0, true), (1, false), (2, true), (60, false)],
        );
        assert_eq!(test.total_bounces(), 0);
        assert!(!test.bounce_measurable());

        let results = test.get_results();
        let bounce = row(&results, "Bounces Detected");
        assert_eq!(bounce.status, ResultStatus::Info);
        assert!(bounce.value.contains("n/a"));
        assert!(!results.iter().any(|r| r.status == ResultStatus::Ok));
    }

    #[test]
    fn press_only_input_does_not_invent_bounce_or_hold_times() {
        // Synthesized release at the top of the loop and a re-press handled
        // right after the draw used to land inside the window.
        let mut test = HoldReleaseTest::default();
        test.set_input_source(InputSource::TerminalPressOnly);
        let t0 = Instant::now();
        feed(&mut test, KEY, t0, &[(0, true), (120, false), (123, true)]);
        assert_eq!(test.total_bounces(), 0);
        let results = test.get_results();
        assert!(row(&results, "Bounces Detected").value.contains("n/a"));
        assert!(row(&results, "Avg Hold Time").value.contains("n/a"));
    }

    #[test]
    fn nothing_pressed_is_not_reported_as_clean() {
        let test = HoldReleaseTest::default();
        let bounce = row(&test.get_results(), "Bounces Detected").clone();
        assert_eq!(bounce.status, ResultStatus::Info);
    }

    #[test]
    fn hold_time_is_measured_with_real_releases() {
        let mut test = HoldReleaseTest::default();
        let t0 = Instant::now();
        feed(&mut test, KEY, t0, &[(0, true), (80, false)]);
        assert!((test.avg_hold_ms().unwrap() - 80.0).abs() < 0.5);
        assert_eq!(
            test.min_hold_by_key(),
            vec![(KEY, test.avg_hold_ms().unwrap())]
        );
    }

    #[test]
    fn reset_keeps_settings() {
        let mut test = HoldReleaseTest::new(30);
        test.set_input_source(InputSource::TerminalWithRelease);
        let t0 = Instant::now();
        feed(&mut test, KEY, t0, &[(0, true), (50, false)]);
        test.reset();
        assert_eq!(test.total_presses(), 0);
        assert_eq!(test.bounce_window_ms(), 30);
        assert!(!test.bounce_measurable());
    }

    #[test]
    fn is_chatter_press_rules() {
        let t0 = Instant::now();
        let w = ms(DEFAULT_BOUNCE_WINDOW_MS);
        // first press of a key
        assert!(!is_chatter_press(t0, None, None, w));
        // 19 ms after release: chatter; 21 ms (with a long press before): not
        assert!(is_chatter_press(
            t0 + ms(119),
            Some(t0),
            Some(t0 + ms(100)),
            w
        ));
        assert!(!is_chatter_press(
            t0 + ms(121),
            Some(t0),
            Some(t0 + ms(100)),
            w
        ));
        // 39 ms press-to-press: chatter; 41 ms: not
        assert!(is_chatter_press(
            t0 + ms(39),
            Some(t0),
            Some(t0 + ms(10)),
            w
        ));
        assert!(!is_chatter_press(
            t0 + ms(41),
            Some(t0),
            Some(t0 + ms(10)),
            w
        ));
    }
}
