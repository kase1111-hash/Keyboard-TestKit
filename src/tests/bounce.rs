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

/// Two presses of the same key closer than this are suspect whatever the
/// release in between looked like: 40 ms is a 25 Hz tap rate, while the
/// fastest same-key tapping is ~10-15 Hz (65 ms or more per tap).
pub const CHATTER_PRESS_GAP_MS: u64 = 40;

/// A key down for less than this was not held by a finger. Deliberate
/// keystrokes stay down for tens of ms: typists' shortest holds are ~30-40
/// ms, and even two-finger drumming on one key at ~15 Hz keeps it down
/// 25 ms or more. Chatter that gets past firmware debounce closes the
/// contact for one or two reports: 1-15 ms at 1000 Hz (past a ~5 ms
/// debounce), 8 or 16 ms at 125 Hz.
///
/// A suspect re-press is only chatter when the keystroke before it or the
/// re-press itself is this short. A radio link (Bluetooth, a 2.4 GHz
/// receiver) can hold a release back and deliver it together with the next
/// press, so a fast double letter arrives with no gap at all; its two holds
/// are still a finger's.
pub const MIN_FINGER_HOLD_MS: u64 = 20;

/// Whether a suspect re-press (see [`is_chatter_press`]) shows a switch
/// artefact: a contact no finger makes, either the keystroke that ended just
/// before it (`previous_hold`, a press bounce) or its own (`repress_hold`, a
/// release bounce). `None` is a hold not known (yet): the re-press is still
/// down, and one already held [`MIN_FINGER_HOLD_MS`] is a finger's.
pub fn is_switch_chatter(previous_hold: Option<Duration>, repress_hold: Option<Duration>) -> bool {
    let short = |hold: Duration| hold < Duration::from_millis(MIN_FINGER_HOLD_MS);
    previous_hold.is_some_and(short) || repress_hold.is_some_and(short)
}

/// Whether `bounces` on one key, `over_wireless` of them counted from events
/// that came from a wireless keyboard, are a switch fault rather than
/// something to check again. Once is worth a look but not yet a pattern.
/// Over a wireless link nothing is conclusive: a radio stall that ends by
/// delivering a whole keystroke (release, press, release) at once has the
/// shape of a release bounce, and a flaky link stalls again and again.
pub fn bounces_are_fault(bounces: u32, over_wireless: u32) -> bool {
    bounces.saturating_sub(over_wireless) >= 2
}

/// A key whose keystrokes are all shorter than this is released by the
/// kernel driver, not by the key: laptop keys with an atkbd "force release"
/// quirk and WMI hotkeys get a release microseconds after every press, and
/// repeat as press/release pairs ~33 ms apart while held. A real release
/// needs another report (a PS/2 break code takes over 1 ms, a 1000 Hz USB
/// poll 1 ms), so any normal keystroke of a key clears this.
const MIN_REPORTED_HOLD: Duration = Duration::from_micros(500);

/// Whether a press re-registers the same key too soon to be a new keystroke
/// arriving on time: a suspect, which [`is_switch_chatter`] then judges from
/// the holds around it.
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
    /// How long the previous keystroke of this key was held
    last_hold: Option<Duration>,
    /// Whether the key reports its own releases (a keystroke lasted at least
    /// [`MIN_REPORTED_HOLD`]); keys the driver auto-releases are not judged
    reports_releases: bool,
    /// The current press is a suspect re-press after a finger-length
    /// keystroke: chatter only if its own hold turns out short
    suspect: bool,
    /// Bounces counted from events that came from a wireless keyboard
    wireless_bounces: u32,
}

impl KeyHoldStats {
    /// Whether the key bounced, but only in events that came from a wireless
    /// keyboard, which a radio stall can explain.
    fn bounced_only_over_wireless(&self) -> bool {
        self.bounce_count > 0 && self.wireless_bounces == self.bounce_count
    }
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
    /// Last press of any key (for the press rate)
    last_event_time: Option<Instant>,
    /// Gaps under 500 ms between presses of any keys. This is how fast keys
    /// are pressed, not autorepeat: evdev and the terminal listener drop
    /// autorepeat, so holding a key adds nothing here.
    press_intervals: VecDeque<u64>,
    /// Keys whose events, until the next
    /// [`HoldReleaseTest::note_wireless_keys`], come from a wireless keyboard
    wireless_keys: Vec<KeyCode>,
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
            press_intervals: VecDeque::new(),
            wireless_keys: Vec::new(),
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

    /// Presses per second (of any keys) while keys are pressed in quick
    /// succession: typing pace, not the keyboard's autorepeat rate.
    pub fn press_rate_hz(&self) -> Option<f64> {
        if self.press_intervals.len() < 2 {
            return None;
        }
        let avg_interval_us =
            self.press_intervals.iter().sum::<u64>() as f64 / self.press_intervals.len() as f64;
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

    /// Say which keys' events, from now until the next call, come from a
    /// wireless keyboard (Bluetooth or a 2.4 GHz receiver), whose radio can
    /// deliver reports late and together. Called before each batch of
    /// events, so a bounce is put down to the link it came over: the same
    /// key typed on a cable or a laptop's own keyboard is still judged.
    pub fn note_wireless_keys(&mut self, keys: &[KeyCode]) {
        self.wireless_keys.clear();
        self.wireless_keys.extend_from_slice(keys);
    }

    /// How many of `key`'s bounces were counted from events that came from a
    /// wireless keyboard (see [`HoldReleaseTest::note_wireless_keys`]).
    pub fn wireless_bounces(&self, key: KeyCode) -> u32 {
        self.key_stats.get(&key).map_or(0, |s| s.wireless_bounces)
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
        let over_wireless = self.wireless_keys.contains(&event.key);
        let stats = self.key_stats.entry(event.key).or_default();

        match event.event_type {
            KeyEventType::Press => {
                // A bounce is the switch registering the same key again too
                // soon after its release/press. Terminal timestamps are read
                // times, so this is only judged with device timestamps, and
                // only for keys whose releases come from the key itself.
                stats.suspect = false;
                if check_bounce
                    && stats.reports_releases
                    && is_chatter_press(
                        event.timestamp,
                        stats.last_press,
                        stats.last_release,
                        bounce_window,
                    )
                {
                    if is_switch_chatter(stats.last_hold, None) {
                        stats.bounce_count += 1;
                        stats.wireless_bounces += u32::from(over_wireless);
                        self.total_bounces += 1;
                    } else {
                        // After a finger's keystroke this may be a release
                        // the radio delivered late: its own hold decides
                        stats.suspect = true;
                    }
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

                // Track press intervals (pauses of 500 ms or more are not
                // part of a burst of presses)
                if let Some(last) = self.last_event_time {
                    let interval = event.timestamp.duration_since(last).as_micros() as u64;
                    if interval < 500_000 {
                        self.press_intervals.push_back(interval);
                        if self.press_intervals.len() > 100 {
                            self.press_intervals.pop_front();
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
                    let hold = event.timestamp.saturating_duration_since(start);
                    if hold >= MIN_REPORTED_HOLD {
                        stats.reports_releases = true;
                    }
                    if std::mem::take(&mut stats.suspect) && is_switch_chatter(None, Some(hold)) {
                        stats.bounce_count += 1;
                        stats.wireless_bounces += u32::from(over_wireless);
                        self.total_bounces += 1;
                    }
                    stats.last_hold = Some(hold);
                    let hold_ms = hold.as_secs_f64() * 1000.0;
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
            } else if self
                .key_stats
                .values()
                .all(|s| s.bounce_count == 0 || s.bounced_only_over_wireless())
            {
                // A radio stall can look the same: not a verdict on the switch
                results.push(TestResult::warning(
                    "Bounces Detected",
                    format!("{} (wireless link)", self.total_bounces),
                ));
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
            // A late radio report can close the gap, not shorten a finger's hold
            results.push(TestResult::info(
                "Counted only if",
                format!(
                    "the hold before or after it is <{} ms (no finger)",
                    MIN_FINGER_HOLD_MS
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

        // How fast keys were pressed (typing pace: autorepeat is not
        // reported by any input path, so this is not a repeat rate)
        if let Some(rate) = self.press_rate_hz() {
            results.push(TestResult::info(
                "Press Rate (any keys)",
                format!("{:.1} presses/sec", rate),
            ));
        }

        // Bouncy keys
        let bouncy = self.bouncy_keys();
        if !bouncy.is_empty() {
            results.push(TestResult::warning("--- Bouncy Keys ---", ""));
            for (key, count) in bouncy.iter().take(5) {
                let key_info = keymap::get_key_info(*key);
                let label = format!("  {}", key_info.name);
                let over_wireless = self
                    .key_stats
                    .get(key)
                    .is_some_and(KeyHoldStats::bounced_only_over_wireless);
                results.push(if over_wireless {
                    TestResult::warning(label, format!("{} bounces (wireless)", count))
                } else {
                    TestResult::error(label, format!("{} bounces", count))
                });
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
        self.press_intervals.clear();
        self.wireless_keys.clear();
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
        // Worn switch on a 125 Hz keyboard: as the key comes up the contact
        // re-closes two reports (16 ms) after the release, for one report
        // (8 ms). The old 5 ms window could never see it.
        let mut test = HoldReleaseTest::default();
        let t0 = Instant::now();
        feed(
            &mut test,
            KEY,
            t0,
            &[(0, true), (72, false), (88, true), (96, false)],
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
    fn release_and_press_bounce_are_detected_at_1000_and_125_hz() {
        // (events, what happened): every timeline is one keystroke whose
        // switch closed again for a moment, on a 1 ms or an 8 ms report grid
        let cases: [(&[(u64, bool)], &str); 6] = [
            (
                &[(0, true), (85, false), (88, true), (91, false)],
                "1000 Hz release bounce: re-closed 3 ms after the release, for 3 ms",
            ),
            (
                &[(0, true), (3, false), (7, true), (90, false)],
                "1000 Hz press bounce: opened 3 ms after the press, for 4 ms",
            ),
            (
                &[(0, true), (12, false), (18, true), (95, false)],
                "1000 Hz bottom-out chatter 12 ms into the press",
            ),
            (
                &[(0, true), (72, false), (80, true), (88, false)],
                "125 Hz release bounce, one report each",
            ),
            (
                &[(0, true), (72, false), (88, true), (104, false)],
                "125 Hz release bounce, two reports each",
            ),
            (
                &[(0, true), (8, false), (16, true), (96, false)],
                "125 Hz press bounce",
            ),
        ];
        for (events, what) in cases {
            let mut test = HoldReleaseTest::default();
            feed(&mut test, KEY, Instant::now(), events);
            assert_eq!(test.bouncy_keys(), vec![(KEY, 1)], "{}", what);
        }
    }

    #[test]
    fn a_fast_re_press_is_judged_when_it_ends() {
        // A re-press 2 ms after a 80 ms keystroke: a release bounce if it
        // ends at once, a late radio report if a finger holds it
        let mut test = HoldReleaseTest::default();
        let t0 = Instant::now();
        feed(&mut test, KEY, t0, &[(0, true), (80, false), (82, true)]);
        assert_eq!(test.total_bounces(), 0, "not judged while down");
        feed(&mut test, KEY, t0, &[(85, false)]);
        assert_eq!(test.total_bounces(), 1);

        let mut test = HoldReleaseTest::default();
        feed(
            &mut test,
            KEY,
            t0,
            &[(0, true), (80, false), (82, true), (140, false)],
        );
        assert_eq!(test.total_bounces(), 0);
        // A chain of blips counts every extra registration
        let mut test = HoldReleaseTest::default();
        feed(
            &mut test,
            KEY,
            t0,
            &[
                (0, true),
                (60, false),
                (62, true),
                (64, false),
                (66, true),
                (68, false),
                (70, true),
                (130, false),
            ],
        );
        assert_eq!(test.total_bounces(), 3);
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
    fn radio_bunched_double_tap_is_not_bounce() {
        // "ee" typed on a Bluetooth keyboard: the first release was held
        // back by the radio and arrived 0.2 ms before the next press. Both
        // keystrokes are a finger's (60+ ms down), so nothing chattered.
        let mut test = HoldReleaseTest::default();
        test.note_wireless_keys(&[KEY]);
        let t0 = Instant::now();
        let us = Duration::from_micros;
        test.process_event(&press_at(KEY, t0, 0));
        test.process_event(&release_at(KEY, t0 + us(109_800)));
        test.process_event(&press_at(KEY, t0 + us(110_000), 0));
        test.process_event(&release_at(KEY, t0 + ms(175)));
        assert_eq!(test.total_bounces(), 0, "{:?}", test.bouncy_keys());

        // The same over a 125 Hz 2.4 GHz receiver, which hands over one
        // queued report per poll: release and press 8 ms apart
        let receiver_tap = [(0, true), (112, false), (120, true), (184, false)];
        feed(&mut test, KEY, t0 + ms(1000), &receiver_tap);
        assert_eq!(test.total_bounces(), 0, "{:?}", test.bouncy_keys());
        // Whatever the link: on a receiver taken for a cable it is not
        // judged either
        test.note_wireless_keys(&[]);
        feed(&mut test, KEY, t0 + ms(2000), &receiver_tap);
        assert_eq!(
            row(&test.get_results(), "Bounces Detected").status,
            ResultStatus::Ok
        );
    }

    #[test]
    fn radio_bunched_fast_drumming_is_not_bounce() {
        // Two fingers drumming one key at 15 Hz (67 ms per tap, 27 ms down),
        // about the fastest anyone repeats a key, over a radio that delivers
        // every release late, with the next press (0.1 ms before it). Each
        // tap shows up as a 67 ms hold with no gap.
        let mut test = HoldReleaseTest::default();
        let t0 = Instant::now();
        let us = Duration::from_micros;
        for i in 0..30u64 {
            let down = t0 + ms(i * 67);
            test.process_event(&press_at(KEY, down, 0));
            let up = if i % 3 == 2 {
                down + ms(27) // this one came through on time
            } else {
                down + ms(67) - us(100)
            };
            test.process_event(&release_at(KEY, up));
        }
        assert_eq!(test.total_bounces(), 0, "{:?}", test.bouncy_keys());
    }

    #[test]
    fn a_radio_stall_that_delivers_a_whole_keystroke_is_not_a_verdict() {
        // A 2.4 GHz link stalls for ~110 ms during fast tapping and then
        // delivers release, press and release together. That has the shape
        // of a release bounce, so it is counted, but over a wireless link it
        // is only something to check, never a fault however often it happens.
        let mut test = HoldReleaseTest::default();
        test.note_wireless_keys(&[KEY]);
        let t0 = Instant::now();
        let us = Duration::from_micros;
        for stall in 0..3u64 {
            let s = t0 + ms(stall * 1000);
            test.process_event(&press_at(KEY, s, 0));
            test.process_event(&release_at(KEY, s + ms(150)));
            test.process_event(&press_at(KEY, s + ms(150) + us(80), 0));
            test.process_event(&release_at(KEY, s + ms(150) + us(160)));
        }
        assert_eq!(test.bouncy_keys(), vec![(KEY, 3)]);
        assert_eq!(test.wireless_bounces(KEY), 3);
        assert!(!bounces_are_fault(3, 3));
        let results = test.get_results();
        let bounces = row(&results, "Bounces Detected");
        assert_eq!(bounces.status, ResultStatus::Warning);
        assert!(bounces.value.contains("wireless"), "{:?}", bounces);
        assert!(!results.iter().any(|r| r.status == ResultStatus::Error));

        // A key typed on a wired keyboard next to it is still judged
        feed(
            &mut test,
            KeyCode(30),
            t0 + ms(5000),
            &[(0, true), (80, false), (82, true), (84, false)],
        );
        feed(
            &mut test,
            KeyCode(30),
            t0 + ms(5300),
            &[(0, true), (2, false), (5, true), (84, false)],
        );
        assert_eq!(test.wireless_bounces(KeyCode(30)), 0);
        assert_eq!(
            row(&test.get_results(), "Bounces Detected").status,
            ResultStatus::Error
        );

        // Reset (a retest on the cable) forgets the link
        test.reset();
        feed(
            &mut test,
            KEY,
            t0 + ms(6000),
            &[(0, true), (80, false), (82, true), (84, false)],
        );
        assert_eq!(test.wireless_bounces(KEY), 0);
        assert_eq!(
            row(&test.get_results(), "Bounces Detected").status,
            ResultStatus::Error
        );
    }

    #[test]
    fn bounces_are_put_down_to_the_link_they_came_over() {
        // A laptop with a Bluetooth keyboard: E typed on the Bluetooth one
        // through a radio stall, then on the laptop's own keyboard, whose
        // switch chatters twice. The radio does not excuse the laptop's key.
        let mut test = HoldReleaseTest::default();
        let t0 = Instant::now();
        let us = Duration::from_micros;
        test.note_wireless_keys(&[KEY]);
        test.process_event(&press_at(KEY, t0, 0));
        test.process_event(&release_at(KEY, t0 + ms(150)));
        test.process_event(&press_at(KEY, t0 + ms(150) + us(80), 0));
        test.process_event(&release_at(KEY, t0 + ms(150) + us(160)));
        assert_eq!(test.wireless_bounces(KEY), 1);
        let results = test.get_results();
        assert_eq!(
            row(&results, "Bounces Detected").status,
            ResultStatus::Warning
        );

        // Later polls bring events of the laptop keyboard only
        test.note_wireless_keys(&[]);
        let chatter = [(0, true), (80, false), (82, true), (85, false)];
        feed(&mut test, KEY, t0 + ms(1000), &chatter);
        assert!(!bounces_are_fault(2, test.wireless_bounces(KEY)));
        feed(&mut test, KEY, t0 + ms(2000), &chatter);
        assert_eq!(test.bouncy_keys(), vec![(KEY, 3)]);
        assert_eq!(test.wireless_bounces(KEY), 1);
        assert!(bounces_are_fault(3, 1));
        let results = test.get_results();
        assert_eq!(
            row(&results, "Bounces Detected").status,
            ResultStatus::Error
        );
        let key_row = row(&results, &format!("  {}", keymap::get_key_info(KEY).name));
        assert_eq!(key_row.status, ResultStatus::Error);
        assert!(!key_row.value.contains("wireless"), "{:?}", key_row);
    }

    #[test]
    fn switch_chatter_rules() {
        let short = Some(ms(MIN_FINGER_HOLD_MS - 1));
        let finger = Some(ms(MIN_FINGER_HOLD_MS));
        // press bounce: the keystroke before the re-press was a blip
        assert!(is_switch_chatter(short, None));
        // release bounce: the re-press itself is a blip
        assert!(is_switch_chatter(finger, short));
        // two holds a finger makes, however close together they arrived
        assert!(!is_switch_chatter(finger, finger));
        // still down, after a finger's keystroke: nothing to say yet
        assert!(!is_switch_chatter(finger, None));
        assert!(!is_switch_chatter(None, None));

        assert!(!bounces_are_fault(1, 0));
        assert!(bounces_are_fault(2, 0));
        // only the bounces that came over a cable count towards a fault
        assert!(!bounces_are_fault(2, 1));
        assert!(!bounces_are_fault(5, 5));
        assert!(bounces_are_fault(5, 3));
    }

    #[test]
    fn driver_auto_released_keys_are_not_bounce() {
        // A laptop volume key with an atkbd force-release quirk, held for a
        // second: typematic make codes every 33 ms, each released by the
        // driver a few µs later. The key never reports its own release.
        let mut test = HoldReleaseTest::default();
        let vol = KeyCode(115);
        let t0 = Instant::now();
        let mut at = t0;
        for i in 0..25u64 {
            test.process_event(&press_at(vol, at, 0));
            test.process_event(&release_at(vol, at + Duration::from_micros(5)));
            at += if i == 0 {
                ms(250)
            } else {
                Duration::from_micros(33_333)
            };
        }
        assert_eq!(test.total_bounces(), 0, "{:?}", test.bouncy_keys());

        // A normal key chattering on its very first keystroke is still caught
        feed(
            &mut test,
            KEY,
            t0 + ms(2000),
            &[(0, true), (1, false), (3, true), (80, false)],
        );
        assert_eq!(test.bouncy_keys(), vec![(KEY, 1)]);
    }

    #[test]
    fn old_five_ms_setting_is_raised_to_a_useful_window() {
        let mut test = HoldReleaseTest::new(5);
        assert_eq!(test.bounce_window_ms(), MIN_BOUNCE_WINDOW_MS);
        let t0 = Instant::now();
        feed(
            &mut test,
            KEY,
            t0,
            &[(0, true), (70, false), (82, true), (90, false)],
        );
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
        feed(
            &mut test,
            KEY,
            t0,
            &[(0, true), (60, false), (85, true), (93, false)],
        );
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
    fn press_rate_is_typing_pace_not_autorepeat() {
        // Typing three different keys 100 ms apart: 10 presses per second,
        // labelled as what it is
        let mut test = HoldReleaseTest::default();
        let t0 = Instant::now();
        for (i, key) in [30u16, 31, 32].iter().enumerate() {
            feed(
                &mut test,
                KeyCode(*key),
                t0 + ms(i as u64 * 100),
                &[(0, true), (50, false)],
            );
        }
        assert!((test.press_rate_hz().unwrap() - 10.0).abs() < 0.01);
        let results = test.get_results();
        assert_eq!(
            row(&results, "Press Rate (any keys)").value,
            "10.0 presses/sec"
        );
        assert!(!results.iter().any(|r| r.label.contains("Repeat")));
        // Holding a key adds nothing: its autorepeat never arrives
        let mut test = HoldReleaseTest::default();
        feed(&mut test, KEY, t0, &[(0, true), (1500, false)]);
        assert!(test.press_rate_hz().is_none());
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
