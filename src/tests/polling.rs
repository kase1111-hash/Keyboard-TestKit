//! Polling rate test module

use super::{InputSource, KeyboardTest, ResultStatus, TestResult};
use crate::keyboard::{KeyEvent, KeyEventType};
use crate::utils::MinMaxExt;
use std::time::{Duration, Instant};

/// Test for measuring keyboard polling rate
pub struct PollingRateTest {
    /// Test duration
    duration: Duration,
    /// Sample window for windowed average (in microseconds)
    sample_window_us: u64,
    /// Start time of the test
    start_time: Option<Instant>,
    /// Recorded intervals between key presses (in microseconds), for the
    /// key-rate statistics
    intervals_us: Vec<u64>,
    /// Gaps between consecutive key events of any kind (presses and
    /// releases), for the report-grid estimate (see
    /// [`PollingRateTest::estimated_poll_interval_us`])
    report_gaps_us: Vec<u64>,
    /// Last press timestamp
    last_event: Option<Instant>,
    /// Last key event (press or release) timestamp
    last_any_event: Option<Instant>,
    /// Number of events recorded
    event_count: u64,
    /// Minimum interval seen
    min_interval_us: Option<u64>,
    /// Maximum interval seen
    max_interval_us: Option<u64>,
    /// Whether event timestamps come from the device (needed for the estimate)
    precise_timestamps: bool,
}

impl PollingRateTest {
    pub fn new(duration_secs: u64, sample_window_ms: u64) -> Self {
        Self {
            duration: Duration::from_secs(duration_secs),
            sample_window_us: sample_window_ms * 1000,
            start_time: None,
            intervals_us: Vec::with_capacity(10000),
            report_gaps_us: Vec::new(),
            last_event: None,
            last_any_event: None,
            event_count: 0,
            min_interval_us: None,
            max_interval_us: None,
            precise_timestamps: true,
        }
    }

    /// Tell the test where events come from. The poll-rate estimate needs
    /// device (kernel) timestamps; terminal input is stamped when it is read.
    pub fn set_input_source(&mut self, source: InputSource) {
        self.precise_timestamps = source.has_precise_timestamps();
    }

    /// Change the test duration (used for the progress display).
    pub fn set_duration_secs(&mut self, duration_secs: u64) {
        self.duration = Duration::from_secs(duration_secs);
    }

    /// The test duration in seconds.
    pub fn duration_secs(&self) -> u64 {
        self.duration.as_secs()
    }

    /// Whether the poll rate can be estimated with the current input source.
    pub fn poll_rate_measurable(&self) -> bool {
        self.precise_timestamps
    }

    /// Calculate average polling rate in Hz (over all samples)
    pub fn avg_rate_hz(&self) -> Option<f64> {
        if self.intervals_us.is_empty() {
            return None;
        }
        let avg_us: f64 =
            self.intervals_us.iter().sum::<u64>() as f64 / self.intervals_us.len() as f64;
        if avg_us > 0.0 {
            Some(1_000_000.0 / avg_us)
        } else {
            None
        }
    }

    /// Calculate windowed average polling rate in Hz.
    ///
    /// Only considers the most recent samples that fit within
    /// `sample_window_us`. Falls back to the global average if
    /// no samples fall within the window.
    pub fn windowed_rate_hz(&self) -> Option<f64> {
        if self.intervals_us.is_empty() || self.sample_window_us == 0 {
            return self.avg_rate_hz();
        }

        // Walk backwards, summing intervals until we exceed the window
        let mut sum: u64 = 0;
        let mut count: usize = 0;
        for &interval in self.intervals_us.iter().rev() {
            if sum + interval > self.sample_window_us && count > 0 {
                break;
            }
            sum += interval;
            count += 1;
        }

        if count == 0 || sum == 0 {
            return self.avg_rate_hz();
        }

        let avg_us = sum as f64 / count as f64;
        Some(1_000_000.0 / avg_us)
    }

    /// Calculate minimum polling rate
    pub fn min_rate_hz(&self) -> Option<f64> {
        self.max_interval_us.map(|us| 1_000_000.0 / us as f64)
    }

    /// Calculate maximum polling rate
    pub fn max_rate_hz(&self) -> Option<f64> {
        self.min_interval_us.map(|us| 1_000_000.0 / us as f64)
    }

    /// Get jitter (standard deviation of intervals)
    pub fn jitter_us(&self) -> Option<f64> {
        if self.intervals_us.len() < 2 {
            return None;
        }
        let mean = self.intervals_us.iter().sum::<u64>() as f64 / self.intervals_us.len() as f64;
        let variance = self
            .intervals_us
            .iter()
            .map(|&x| {
                let diff = x as f64 - mean;
                diff * diff
            })
            .sum::<f64>()
            / self.intervals_us.len() as f64;
        Some(variance.sqrt())
    }

    /// Number of key press events recorded.
    pub fn event_count(&self) -> u64 {
        self.event_count
    }

    /// Recorded inter-press intervals in microseconds (only intervals < 100ms).
    pub fn intervals_us(&self) -> &[u64] {
        &self.intervals_us
    }

    /// Number of gaps between key events available to the report-grid
    /// estimate (it needs [`MIN_QUANTIZATION_SAMPLES`]).
    pub fn quantization_samples(&self) -> usize {
        self.report_gaps_us.len()
    }

    /// Estimate the keyboard's USB report interval from timestamp quantization.
    ///
    /// A polled keyboard can only deliver reports on its poll boundary, so the
    /// gaps between its events, presses and releases alike, are (near)
    /// multiples of the poll interval. This looks for the largest candidate
    /// interval that explains almost all of the recorded gaps. Releases count
    /// too: a key tapped with one finger is pressed only every 120-200 ms, but
    /// stays down 50-100 ms and up a similar time. It needs precise (kernel)
    /// timestamps and at least [`MIN_QUANTIZATION_SAMPLES`] gaps; returns
    /// `None` otherwise (always for terminal input), or when no 125/250/500 Hz
    /// grid is visible (interrupt-driven or >= 1000 Hz keyboards, Bluetooth).
    pub fn estimated_poll_interval_us(&self) -> Option<u64> {
        if !self.precise_timestamps {
            return None;
        }
        estimate_quantization_us(&self.report_gaps_us)
    }

    /// Estimated polling rate in Hz derived from [`Self::estimated_poll_interval_us`].
    pub fn estimated_poll_rate_hz(&self) -> Option<f64> {
        self.estimated_poll_interval_us()
            .map(|us| 1_000_000.0 / us as f64)
    }

    /// Get test progress (0.0 to 1.0): the configured duration since the
    /// first press, and with kernel timestamps also enough gaps for the
    /// poll-rate estimate (100% while it still needs taps would contradict it).
    pub fn progress(&self) -> f64 {
        let time = match self.start_time {
            Some(start) => {
                let elapsed = start.elapsed();
                (elapsed.as_secs_f64() / self.duration.as_secs_f64()).min(1.0)
            }
            None => 0.0,
        };
        if self.precise_timestamps {
            let samples =
                (self.report_gaps_us.len() as f64 / MIN_QUANTIZATION_SAMPLES as f64).min(1.0);
            time.min(samples)
        } else {
            time
        }
    }
}

/// Minimum number of samples before quantization analysis is attempted.
pub const MIN_QUANTIZATION_SAMPLES: usize = 24;

/// Candidate report intervals (µs) for common polling rates: 125, 250, 500 Hz.
/// 1000 Hz and above cannot be told apart from interrupt-driven input with the
/// tolerance used here, so they are reported as "not quantized".
const QUANTIZATION_CANDIDATES_US: [u64; 3] = [8000, 4000, 2000];

/// Fraction of intervals that must sit on the grid for a candidate to be accepted.
const QUANTIZATION_MATCH_RATIO: f64 = 0.9;

/// Tolerance around each grid point (USB scheduling jitter is well under this).
const QUANTIZATION_TOLERANCE_US: u64 = 400;

/// Estimate the timestamp quantization step of a set of intervals (see
/// [`PollingRateTest::estimated_poll_interval_us`]).
///
/// A candidate must explain almost every gap, and the gaps must actually
/// fill its grid: gaps that are all multiples of a coarser period are not a
/// report grid but the period itself. A Bluetooth LE keyboard on a 30 ms
/// connection interval delivers presses 30, 60, 90 ms apart, all multiples
/// of 2 ms, and would otherwise be called a 500 Hz keyboard.
pub fn estimate_quantization_us(intervals: &[u64]) -> Option<u64> {
    if intervals.len() < MIN_QUANTIZATION_SAMPLES {
        return None;
    }
    for &step in &QUANTIZATION_CANDIDATES_US {
        let multiples: Vec<u64> = intervals
            .iter()
            .filter(|&&iv| {
                let rem = iv % step;
                rem <= QUANTIZATION_TOLERANCE_US || step - rem <= QUANTIZATION_TOLERANCE_US
            })
            .map(|iv| (iv + step / 2) / step)
            .collect();
        if (multiples.len() as f64) < intervals.len() as f64 * QUANTIZATION_MATCH_RATIO {
            continue;
        }
        let period = multiples.iter().fold(0, |g, &m| gcd(g, m));
        return (period == 1).then_some(step);
    }
    None
}

/// Greatest common divisor (`gcd(0, n) == n`).
fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

impl KeyboardTest for PollingRateTest {
    fn name(&self) -> &'static str {
        "Polling Rate Test"
    }

    fn description(&self) -> &'static str {
        "Measures keyboard polling rate by analyzing event timing"
    }

    fn process_event(&mut self, event: &KeyEvent) {
        // Every key event sits on the report grid. Events reported together
        // (one USB report, the same kernel timestamp) say nothing about its
        // period, so gaps that are practically zero are left out.
        if let Some(last) = self.last_any_event {
            let gap_us = event.timestamp.saturating_duration_since(last).as_micros() as u64;
            if gap_us > QUANTIZATION_TOLERANCE_US && gap_us < 100_000 {
                self.report_gaps_us.push(gap_us);
            }
        }
        self.last_any_event = Some(event.timestamp);

        // The key-rate statistics count key presses only
        if event.event_type != KeyEventType::Press {
            return;
        }

        // Start the test on first event
        if self.start_time.is_none() {
            self.start_time = Some(Instant::now());
        }

        // Record interval from last event
        if let Some(last) = self.last_event {
            let interval_us = event.timestamp.duration_since(last).as_micros() as u64;

            // NOTE: 100ms threshold is arbitrary. Should be configurable or derived
            // from the keyboard's expected polling rate.
            if interval_us < 100_000 {
                // Less than 100ms
                self.intervals_us.push(interval_us);
                self.min_interval_us.update_min(interval_us);
                self.max_interval_us.update_max(interval_us);
            }
        }

        self.last_event = Some(event.timestamp);
        self.event_count += 1;
    }

    fn is_complete(&self) -> bool {
        match self.start_time {
            Some(start) => start.elapsed() >= self.duration,
            None => false,
        }
    }

    fn get_results(&self) -> Vec<TestResult> {
        let mut results = Vec::new();

        // Tooltip: Explain what this test measures
        results.push(TestResult::info("--- What This Measures ---", ""));
        results.push(TestResult::info(
            "Polling rate = how often",
            "keyboard sends data to PC",
        ));
        results.push(TestResult::info(
            "Higher Hz = lower latency",
            "1000Hz=1ms, 125Hz=8ms delay",
        ));
        results.push(TestResult::info(
            "Tap keys quickly; poll rate",
            "is inferred from timestamps",
        ));
        results.push(TestResult::info("", ""));

        results.push(TestResult::info(
            "Events Recorded",
            format!("{}", self.event_count),
        ));

        // Poll rate inferred from timestamp quantization (needs kernel timestamps)
        if !self.precise_timestamps {
            results.push(TestResult::info("Est. Poll Rate", "n/a (terminal input)"));
            results.push(TestResult::info(
                "Terminal timestamps",
                "are read times, too coarse;",
            ));
            results.push(TestResult::info(
                "poll rate needs",
                "evdev (Linux, sudo/input group)",
            ));
        } else if self.report_gaps_us.len() >= MIN_QUANTIZATION_SAMPLES {
            match self.estimated_poll_rate_hz() {
                Some(hz) => {
                    // A lower rate is a spec, not a fault: 125 Hz is the USB
                    // default and fine for typing.
                    let status = if hz >= 500.0 {
                        ResultStatus::Ok
                    } else {
                        ResultStatus::Info
                    };
                    results.push(TestResult::new(
                        "Est. Poll Rate",
                        format!("{:.0} Hz (report every {:.0} ms)", hz, 1000.0 / hz),
                        status,
                    ));
                }
                None => {
                    // No 2/4/8 ms grid: a 1000+ Hz keyboard, but also Bluetooth
                    // (connection intervals of their own), so no rate is claimed.
                    results.push(TestResult::info(
                        "Est. Poll Rate",
                        "no 125/250/500 Hz grid (1000+ Hz or Bluetooth)",
                    ));
                }
            }
        } else {
            results.push(TestResult::info(
                "Est. Poll Rate",
                format!(
                    "keep tapping: {} of {} key-event gaps",
                    self.report_gaps_us.len(),
                    MIN_QUANTIZATION_SAMPLES
                ),
            ));
        }

        // Windowed keystroke rate (how fast keys are arriving right now)
        let display_rate = self.windowed_rate_hz().or_else(|| self.avg_rate_hz());
        if let Some(rate) = display_rate {
            results.push(TestResult::info(
                "Burst Key Rate",
                format!("{:.1} keys/s", rate),
            ));
        }

        if let Some(avg) = self.avg_rate_hz() {
            results.push(TestResult::info(
                "Avg Key Rate",
                format!("{:.1} keys/s", avg),
            ));
        }

        if let Some(min) = self.min_interval_us {
            results.push(TestResult::info(
                "Shortest Gap",
                format!("{:.2} ms", min as f64 / 1000.0),
            ));
        }

        if let Some(max) = self.max_interval_us {
            results.push(TestResult::info(
                "Longest Gap",
                format!("{:.2} ms", max as f64 / 1000.0),
            ));
        }

        // Spread of inter-key gaps. This reflects typing rhythm far more than
        // the keyboard, so it is informational only.
        if let Some(jitter) = self.jitter_us() {
            results.push(TestResult::info(
                "Gap Spread (σ)",
                format!("{:.1} ms", jitter / 1000.0),
            ));
        }

        results.push(TestResult::info(
            "Progress",
            format!("{:.0}%", self.progress() * 100.0),
        ));

        results
    }

    fn reset(&mut self) {
        self.start_time = None;
        self.intervals_us.clear();
        self.report_gaps_us.clear();
        self.last_event = None;
        self.last_any_event = None;
        self.event_count = 0;
        self.min_interval_us = None;
        self.max_interval_us = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::test_helpers::{press_at, release_at, DEFAULT_KEY};

    #[test]
    fn new_test_has_no_data() {
        let test = PollingRateTest::new(10, 100);
        assert!(test.avg_rate_hz().is_none());
        assert!(test.min_rate_hz().is_none());
        assert!(test.max_rate_hz().is_none());
        assert!(test.jitter_us().is_none());
        assert_eq!(test.progress(), 0.0);
    }

    #[test]
    fn avg_rate_hz_1000hz() {
        let mut test = PollingRateTest::new(10, 100);
        // Manually set intervals of 1000us each = 1000 Hz
        test.intervals_us = vec![1000, 1000, 1000, 1000, 1000];

        let rate = test.avg_rate_hz().unwrap();
        assert!((rate - 1000.0).abs() < 0.01);
    }

    #[test]
    fn avg_rate_hz_125hz() {
        let mut test = PollingRateTest::new(10, 100);
        // 8000us intervals = 125 Hz
        test.intervals_us = vec![8000, 8000, 8000, 8000];

        let rate = test.avg_rate_hz().unwrap();
        assert!((rate - 125.0).abs() < 0.01);
    }

    #[test]
    fn min_max_rate_calculation() {
        let mut test = PollingRateTest::new(10, 100);
        test.min_interval_us = Some(500); // 2000 Hz
        test.max_interval_us = Some(2000); // 500 Hz

        let max_rate = test.max_rate_hz().unwrap();
        assert!((max_rate - 2000.0).abs() < 0.01);

        let min_rate = test.min_rate_hz().unwrap();
        assert!((min_rate - 500.0).abs() < 0.01);
    }

    #[test]
    fn jitter_calculation_uniform() {
        let mut test = PollingRateTest::new(10, 100);
        // All same intervals = 0 jitter
        test.intervals_us = vec![1000, 1000, 1000, 1000, 1000];

        let jitter = test.jitter_us().unwrap();
        assert!(jitter.abs() < 0.01);
    }

    #[test]
    fn jitter_calculation_varied() {
        let mut test = PollingRateTest::new(10, 100);
        // Varied intervals should have non-zero jitter
        test.intervals_us = vec![500, 1500, 500, 1500, 500, 1500];

        let jitter = test.jitter_us().unwrap();
        assert!(jitter > 0.0);
        // Standard deviation of [500, 1500, ...] with mean 1000 is 500
        assert!((jitter - 500.0).abs() < 1.0);
    }

    #[test]
    fn jitter_requires_two_samples() {
        let mut test = PollingRateTest::new(10, 100);
        test.intervals_us = vec![1000];
        assert!(test.jitter_us().is_none());
    }

    #[test]
    fn process_event_ignores_release() {
        let mut test = PollingRateTest::new(10, 100);
        let now = Instant::now();

        test.process_event(&release_at(DEFAULT_KEY, now));

        assert_eq!(test.event_count, 0);
        assert!(test.start_time.is_none());
    }

    #[test]
    fn process_event_starts_test() {
        let mut test = PollingRateTest::new(10, 100);
        let now = Instant::now();

        test.process_event(&press_at(DEFAULT_KEY, now, 0));

        assert!(test.start_time.is_some());
        assert_eq!(test.event_count, 1);
    }

    #[test]
    fn process_event_filters_large_intervals() {
        let mut test = PollingRateTest::new(10, 100);
        let now = Instant::now();

        // First event
        test.process_event(&press_at(DEFAULT_KEY, now, 0));

        // Simulate a long pause (200ms = 200000us) - this should be filtered
        let later = now + Duration::from_millis(200);
        test.process_event(&press_at(DEFAULT_KEY, later, 0));

        // The large interval should not be recorded
        assert!(test.intervals_us.is_empty());
    }

    #[test]
    fn reset_clears_all() {
        let mut test = PollingRateTest::new(10, 100);
        test.start_time = Some(Instant::now());
        test.intervals_us = vec![1000, 2000];
        test.event_count = 5;
        test.min_interval_us = Some(500);
        test.max_interval_us = Some(2000);

        test.reset();

        assert!(test.start_time.is_none());
        assert!(test.intervals_us.is_empty());
        assert_eq!(test.event_count, 0);
        assert!(test.min_interval_us.is_none());
        assert!(test.max_interval_us.is_none());
    }

    #[test]
    fn is_complete_before_start() {
        let test = PollingRateTest::new(10, 100);
        assert!(!test.is_complete());
    }

    #[test]
    fn test_name_and_description() {
        let test = PollingRateTest::new(10, 100);
        assert_eq!(test.name(), "Polling Rate Test");
        assert!(!test.description().is_empty());
    }

    /// Feed presses separated by the given gaps (µs), rolling over four
    /// keys as fast typing does (one key cannot be re-pressed that fast).
    fn feed_gaps(test: &mut PollingRateTest, gaps_us: &[u64]) {
        let key = |i: usize| crate::keyboard::KeyCode(30 + (i % 4) as u16);
        let mut at = Instant::now();
        test.process_event(&press_at(key(0), at, 0));
        for (i, &gap) in gaps_us.iter().enumerate() {
            at += Duration::from_micros(gap);
            test.process_event(&press_at(key(i + 1), at, gap));
        }
    }

    fn poll_row(test: &PollingRateTest) -> TestResult {
        test.get_results()
            .into_iter()
            .find(|r| r.label == "Est. Poll Rate")
            .expect("poll rate row")
    }

    /// Fast rolled typing on a 125 Hz keyboard: human gaps of 24-56 ms,
    /// each one landing on the 8 ms report grid.
    fn gaps_125hz() -> Vec<u64> {
        (0..30u64).map(|i| 8_000 * (3 + i % 4)).collect()
    }

    /// Fast typing with gaps that are not on any report grid (1000 Hz
    /// keyboard, or Bluetooth with jittery connection events).
    fn gaps_unquantized() -> Vec<u64> {
        (0..30u64).map(|i| 31_000 + (i * 7_919) % 23_000).collect()
    }

    #[test]
    fn quantized_evdev_timestamps_give_the_poll_rate() {
        let mut test = PollingRateTest::new(10, 100);
        feed_gaps(&mut test, &gaps_125hz());
        assert_eq!(test.estimated_poll_interval_us(), Some(8000));
        let row = poll_row(&test);
        assert!(row.value.starts_with("125 Hz"), "{:?}", row);
        // 125 Hz is the USB default, not a fault
        assert_eq!(row.status, ResultStatus::Info);
    }

    #[test]
    fn no_grid_is_not_claimed_as_1000hz() {
        let mut test = PollingRateTest::new(10, 100);
        feed_gaps(&mut test, &gaps_unquantized());
        assert_eq!(test.estimated_poll_interval_us(), None);
        let row = poll_row(&test);
        assert_eq!(row.status, ResultStatus::Info, "{:?}", row);
        assert!(!row.value.contains(">= 1000"), "{:?}", row);
    }

    #[test]
    fn terminal_input_reports_poll_rate_as_not_measurable() {
        for source in [
            InputSource::TerminalWithRelease,
            InputSource::TerminalPressOnly,
        ] {
            for gaps in [gaps_125hz(), gaps_unquantized()] {
                let mut test = PollingRateTest::new(10, 100);
                test.set_input_source(source);
                feed_gaps(&mut test, &gaps);
                assert!(!test.poll_rate_measurable());
                assert_eq!(test.estimated_poll_interval_us(), None);
                assert_eq!(test.estimated_poll_rate_hz(), None);
                let row = poll_row(&test);
                assert_eq!(row.status, ResultStatus::Info);
                assert!(row.value.contains("n/a"), "{:?}", row);
                assert!(!test
                    .get_results()
                    .iter()
                    .any(|r| r.status == ResultStatus::Ok));
            }
        }
    }

    #[test]
    fn evdev_input_keeps_the_estimate() {
        let mut test = PollingRateTest::new(10, 100);
        test.set_input_source(InputSource::Evdev);
        feed_gaps(&mut test, &gaps_125hz());
        assert!(test.poll_rate_measurable());
        assert_eq!(test.estimated_poll_interval_us(), Some(8000));
    }

    /// Taps of one key with one finger, ~6 per second: presses 150-190 ms
    /// apart (too far apart to count on their own), each held 60-90 ms, all
    /// on a report grid of `step_us`.
    fn one_finger_taps(step_us: u64, taps: u64) -> Vec<KeyEvent> {
        let on_grid = |us: u64| us / step_us * step_us;
        let mut at = Instant::now();
        let mut events = Vec::new();
        for i in 0..taps {
            let hold = on_grid(60_000 + (i * 7_919) % 30_000);
            let up = on_grid(90_000 + (i * 4_513) % 40_000);
            events.push(press_at(DEFAULT_KEY, at, 0));
            events.push(release_at(DEFAULT_KEY, at + Duration::from_micros(hold)));
            at += Duration::from_micros(hold + up);
        }
        events
    }

    #[test]
    fn one_finger_rapid_taps_give_the_poll_rate() {
        // The auto test's rapid-tap step: no two presses are under 100 ms
        // apart, but the holds and the gaps between taps are on the grid
        for (step, hz) in [(8_000, "125 Hz"), (4_000, "250 Hz")] {
            let mut test = PollingRateTest::new(10, 100);
            for event in one_finger_taps(step, 30) {
                test.process_event(&event);
            }
            assert!(test.intervals_us().is_empty());
            assert!(test.quantization_samples() >= MIN_QUANTIZATION_SAMPLES);
            assert_eq!(test.estimated_poll_interval_us(), Some(step));
            assert!(
                poll_row(&test).value.starts_with(hz),
                "{:?}",
                poll_row(&test)
            );
        }
    }

    #[test]
    fn bluetooth_connection_interval_is_not_a_poll_rate() {
        // A BLE keyboard on a 30 ms (or 20/10/40 ms) connection interval
        // delivers every report at a connection event: all gaps are
        // multiples of the interval, so also of 2, 4 or 8 ms
        for interval_ms in [10u64, 20, 30, 40] {
            let gaps: Vec<u64> = (0..40u64)
                .map(|i| interval_ms * 1000 * (1 + i % 3))
                .filter(|us| *us < 100_000)
                .collect();
            assert!(gaps.len() >= MIN_QUANTIZATION_SAMPLES);
            assert_eq!(estimate_quantization_us(&gaps), None, "{interval_ms} ms");
        }
        // A real 500 Hz grid filled by human timing is still recognised
        let gaps: Vec<u64> = (0..40u64).map(|i| 2_000 * (11 + (i * 7) % 23)).collect();
        assert_eq!(estimate_quantization_us(&gaps), Some(2_000));
    }

    #[test]
    fn progress_waits_for_enough_samples() {
        let mut test = PollingRateTest::new(10, 100);
        test.process_event(&press_at(DEFAULT_KEY, Instant::now(), 0));
        test.start_time = Some(Instant::now() - Duration::from_secs(30));
        // The time is up, but the estimate still needs gaps
        assert_eq!(test.progress(), 0.0);
        feed_gaps(&mut test, &gaps_125hz());
        assert_eq!(test.progress(), 1.0);
    }

    #[test]
    fn duration_setter_changes_progress() {
        let mut test = PollingRateTest::new(10, 100);
        feed_gaps(&mut test, &gaps_125hz());
        test.start_time = Some(Instant::now() - Duration::from_secs(5));
        assert!((test.progress() - 0.5).abs() < 0.05);
        assert!(!test.is_complete());
        test.set_duration_secs(5);
        assert!(test.is_complete());
        assert!((test.progress() - 1.0).abs() < f64::EPSILON);
    }
}
