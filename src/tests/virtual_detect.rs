//! Virtual keyboard detection and testing module
//!
//! Provides two capabilities:
//! 1. Detection of software-generated keystrokes vs physical input
//! 2. Sending synthetic key events to test software/hardware issues
//!
//! Note: Virtual key sending requires the 'virtual-send' feature and
//! appropriate system libraries (libxdo on Linux, etc.)

use super::{InputSource, KeyboardTest, ResultStatus, TestResult};
use crate::keyboard::{keymap, KeyCode, KeyEvent, KeyEventType};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

/// Thresholds for virtual input detection.
///
/// Different keys pressed within a few ms of each other are normal (chords,
/// Shift+letter, rolled typing, slamming keys in the NKRO test), so timing is
/// only judged for the same key, or for presses made while no other key is
/// held. Synthetic typing releases every key before pressing the next one.
///
/// Same-key re-press faster than this needs a ~65 Hz tap rate; the fastest
/// same-key tapping is ~10-15 Hz.
const MIN_HUMAN_INTERVAL_MS: f64 = 15.0;
/// Interval variance (ms²) below which a sequence is machine-regular: human
/// typing varies by tens of ms between keystrokes (variance in the 100s+).
const PERFECT_TIMING_THRESHOLD: f64 = 0.5;
/// Number of intervals needed before "perfect timing" is judged.
const PERFECT_TIMING_MIN_SAMPLES: usize = 10;
/// Five separate (non-overlapping) keystrokes inside this window would each
/// last under 10 ms from press to next press - no finger does that.
const BURST_WINDOW_MS: u64 = 50;
const BURST_COUNT_THRESHOLD: usize = 5;
const ANALYSIS_WINDOW: usize = 20;

/// Keys sent by the physical-vs-virtual check.
const INJECTED_KEYS: [char; 3] = ['z', 'x', 'c'];
/// How long after the injected keys were sent the check waits for them. The
/// terminal delivers them within a frame or two once the send returns.
const INJECTION_WINDOW: Duration = Duration::from_millis(500);
/// How long before the request a key press still counts as the keystroke
/// that started the check (the 'v' press is stamped just before its command
/// is handled).
const TRIGGER_LOOKBACK: Duration = Duration::from_millis(250);

/// Character a key types, if it is a single-character key (letters, digits).
fn key_char(key: KeyCode) -> Option<char> {
    let label = keymap::get_key_info(key).label;
    let mut chars = label.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c.to_ascii_lowercase()),
        _ => None,
    }
}

/// Whether a key is one the physical-vs-virtual check injects.
fn is_injected_key(key: KeyCode) -> bool {
    key_char(key).is_some_and(|c| INJECTED_KEYS.contains(&c))
}

/// Verdict of the physical-vs-virtual check.
///
/// `physical_seen`: a press of the user's own (non-injected) key, such as the
/// one that started the check, reached the test pipeline. `injected_seen`: at
/// least one injected key arrived. Without a physical press there is nothing
/// to compare against (the app may have been paused), so no hardware or
/// software verdict is guessed.
fn diagnose(physical_seen: bool, injected_seen: bool) -> DiagnosticResult {
    match (physical_seen, injected_seen) {
        (true, true) => DiagnosticResult::KeyboardOk,
        (true, false) => DiagnosticResult::ApiIssue,
        (false, _) => DiagnosticResult::Inconclusive,
    }
}

/// Classification of input source
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputClassification {
    Physical,
    LikelyPhysical,
    Uncertain,
    LikelyVirtual,
    Virtual,
}

impl InputClassification {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Physical => "Physical",
            Self::LikelyPhysical => "Likely Physical",
            Self::Uncertain => "Uncertain",
            Self::LikelyVirtual => "Likely Virtual",
            Self::Virtual => "Virtual/Automated",
        }
    }

    pub fn to_status(self) -> ResultStatus {
        match self {
            Self::Physical | Self::LikelyPhysical => ResultStatus::Ok,
            Self::Uncertain => ResultStatus::Info,
            Self::LikelyVirtual | Self::Virtual => ResultStatus::Warning,
        }
    }
}

/// Diagnostic result for physical vs virtual comparison
///
/// The check can only compare the keystroke that started it with the keys it
/// injects: in terminal mode the starting key press already proves physical
/// input reaches the app, and the terminal cannot tell keyboards apart. So
/// `HardwareIssue` and `SoftwareIssue` are not produced (a missing physical
/// press is reported as `Inconclusive`); per-key hardware faults are found by
/// the auto test's key sweep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticResult {
    NotTested,
    NotAvailable,  // Virtual sending not compiled in, or evdev input
    KeyboardOk,    // Physical works + Virtual works
    HardwareIssue, // Physical fails + Virtual works (not produced, see above)
    SoftwareIssue, // Physical fails + Virtual fails (not produced, see above)
    ApiIssue,      // Physical works + Virtual fails
    Inconclusive,  // No physical key press reached the test during the check
}

impl DiagnosticResult {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotTested => "Not tested yet",
            Self::NotAvailable => "Not available",
            Self::KeyboardOk => "Input path OK",
            Self::HardwareIssue => "Hardware Issue Detected",
            Self::SoftwareIssue => "Software/Driver Issue",
            Self::ApiIssue => "API/Permission Issue",
            Self::Inconclusive => "Inconclusive",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::NotTested => "Press 'v' to send virtual test keys",
            Self::NotAvailable => "Build with --features virtual-send for this test",
            Self::KeyboardOk => "Your key press and the injected keys both reached the app",
            Self::HardwareIssue => "Physical keys fail but virtual works - check keyboard hardware",
            Self::SoftwareIssue => "Both physical and virtual fail - check drivers/software",
            Self::ApiIssue => {
                "Injected keys never arrived - check input-injection permissions and focus"
            }
            Self::Inconclusive => "Your key press did not reach the test (paused?) - try again",
        }
    }

    pub fn to_status(self) -> ResultStatus {
        match self {
            Self::NotTested | Self::NotAvailable | Self::Inconclusive => ResultStatus::Info,
            Self::KeyboardOk => ResultStatus::Ok,
            Self::HardwareIssue => ResultStatus::Error,
            Self::SoftwareIssue => ResultStatus::Error,
            Self::ApiIssue => ResultStatus::Warning,
        }
    }
}

/// Record of a keystroke for analysis
#[derive(Debug, Clone)]
struct KeystrokeRecord {
    _key: KeyCode,
    _timestamp: Instant,
    _interval_ms: Option<f64>,
    _is_virtual: bool,
}

/// Detected anomaly event
#[derive(Debug, Clone)]
pub struct AnomalyEvent {
    pub description: String,
    pub timestamp: Instant,
    pub severity: AnomalySeverity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnomalySeverity {
    Low,
    Medium,
    High,
}

/// Test key result for comparison
#[derive(Debug, Clone, Default)]
struct TestKeyResult {
    virtual_sent: bool,
    virtual_received: bool,
    _last_test_time: Option<Instant>,
}

/// Virtual key sender - conditionally compiled
#[cfg(feature = "virtual-send")]
pub struct VirtualKeySender {
    last_error: Option<String>,
}

#[cfg(feature = "virtual-send")]
impl VirtualKeySender {
    pub fn new() -> Self {
        Self { last_error: None }
    }

    /// Send a virtual key press and release
    pub fn send_key(&mut self, key: char) -> Result<(), String> {
        use enigo::{Enigo, Keyboard, Settings};
        use std::thread;

        let mut enigo =
            Enigo::new(&Settings::default()).map_err(|e| format!("Failed to init: {}", e))?;

        thread::sleep(Duration::from_millis(10));

        enigo
            .key(enigo::Key::Unicode(key), enigo::Direction::Press)
            .map_err(|e| format!("Press failed: {}", e))?;

        thread::sleep(Duration::from_millis(20));

        enigo
            .key(enigo::Key::Unicode(key), enigo::Direction::Release)
            .map_err(|e| format!("Release failed: {}", e))?;

        self.last_error = None;
        Ok(())
    }

    /// Send a sequence of test keys
    pub fn send_test_sequence(&mut self) -> Result<Vec<char>, String> {
        use std::thread;

        let test_keys = vec!['z', 'x', 'c'];

        for &key in &test_keys {
            self.send_key(key)?;
            thread::sleep(Duration::from_millis(100));
        }

        Ok(test_keys)
    }

    pub fn is_available() -> bool {
        true
    }
}

#[cfg(feature = "virtual-send")]
impl Default for VirtualKeySender {
    fn default() -> Self {
        Self::new()
    }
}

/// Stub implementation when virtual-send feature is not enabled
#[cfg(not(feature = "virtual-send"))]
pub struct VirtualKeySender;

#[cfg(not(feature = "virtual-send"))]
impl VirtualKeySender {
    pub fn new() -> Self {
        Self
    }

    pub fn send_test_sequence(&mut self) -> Result<Vec<char>, String> {
        Err("Virtual sending not available - build with --features virtual-send".to_string())
    }

    pub fn is_available() -> bool {
        false
    }
}

#[cfg(not(feature = "virtual-send"))]
impl Default for VirtualKeySender {
    fn default() -> Self {
        Self::new()
    }
}

/// Virtual keyboard detection and testing
pub struct VirtualKeyboardTest {
    /// Recent keystrokes for analysis
    recent_keystrokes: VecDeque<KeystrokeRecord>,
    /// Last keystroke timestamp
    last_keystroke: Option<Instant>,
    /// Total keystrokes analyzed
    total_keystrokes: u64,
    /// Keystrokes classified as likely virtual
    virtual_count: u64,
    /// Detected anomalies
    anomalies: VecDeque<AnomalyEvent>,
    /// Current session classification
    session_classification: InputClassification,
    /// Timing intervals for statistical analysis
    intervals: VecDeque<f64>,
    /// Burst detection window
    burst_window: VecDeque<Instant>,
    /// Test start time
    start_time: Option<Instant>,
    /// Last analysis result
    last_interval_ms: Option<f64>,
    /// Running variance calculation
    interval_variance: f64,
    /// Running mean calculation
    interval_mean: f64,
    /// Non-modifier keys currently held (a press while one is held is a chord)
    held: HashSet<KeyCode>,
    /// Last press time per key (same-key timing)
    last_press_by_key: HashMap<KeyCode, Instant>,

    // Virtual key testing
    /// Virtual key sender
    sender: VirtualKeySender,
    /// Keys we're expecting from virtual send
    expected_virtual_keys: Vec<char>,
    /// Time when the virtual keys had been sent (the wait starts here)
    virtual_send_time: Option<Instant>,
    /// When the check was requested (the 'v' key press)
    requested_at: Option<Instant>,
    /// Latest press of a key the check does not inject
    last_physical_press: Option<Instant>,
    /// Whether injected keys can reach the test (not with evdev input: they go
    /// through the display server, not the keyboard device evdev reads)
    injected_keys_visible: bool,
    /// Results of key comparison tests
    test_results: HashMap<char, TestKeyResult>,
    /// Current diagnostic result
    diagnostic: DiagnosticResult,
    /// Virtual keys received during test
    virtual_keys_received: u32,
    /// Test mode active
    test_mode_active: bool,
    /// Pending virtual send request
    pending_send: bool,
}

impl VirtualKeyboardTest {
    pub fn new() -> Self {
        let diagnostic = if VirtualKeySender::is_available() {
            DiagnosticResult::NotTested
        } else {
            DiagnosticResult::NotAvailable
        };

        Self {
            recent_keystrokes: VecDeque::with_capacity(ANALYSIS_WINDOW + 1),
            last_keystroke: None,
            total_keystrokes: 0,
            virtual_count: 0,
            anomalies: VecDeque::new(),
            session_classification: InputClassification::Uncertain,
            intervals: VecDeque::with_capacity(ANALYSIS_WINDOW + 1),
            burst_window: VecDeque::new(),
            start_time: None,
            last_interval_ms: None,
            interval_variance: 0.0,
            interval_mean: 0.0,
            held: HashSet::new(),
            last_press_by_key: HashMap::new(),
            sender: VirtualKeySender::new(),
            expected_virtual_keys: Vec::new(),
            virtual_send_time: None,
            requested_at: None,
            last_physical_press: None,
            injected_keys_visible: true,
            test_results: HashMap::new(),
            diagnostic,
            virtual_keys_received: 0,
            test_mode_active: false,
            pending_send: false,
        }
    }

    /// Tell the test where events come from. With evdev the injected keys
    /// never reach the test (they enter through the display server, not the
    /// keyboard device), so the physical-vs-virtual check is unavailable.
    pub fn set_input_source(&mut self, source: InputSource) {
        self.injected_keys_visible = source != InputSource::Evdev;
        self.diagnostic = self.idle_diagnostic();
    }

    fn idle_diagnostic(&self) -> DiagnosticResult {
        if VirtualKeySender::is_available() && self.injected_keys_visible {
            DiagnosticResult::NotTested
        } else {
            DiagnosticResult::NotAvailable
        }
    }

    /// Request to send virtual test keys (called from main loop)
    pub fn request_virtual_test(&mut self) {
        if VirtualKeySender::is_available() && self.injected_keys_visible {
            self.pending_send = true;
            self.requested_at = Some(Instant::now());
        }
    }

    /// Check if there's a pending send request
    pub fn has_pending_send(&self) -> bool {
        self.pending_send
    }

    /// Execute the virtual key send (should be called outside of event processing)
    pub fn execute_virtual_send(&mut self) -> Result<(), String> {
        self.pending_send = false;

        if !VirtualKeySender::is_available() {
            return Err("Virtual sending not available".to_string());
        }

        let requested_at = self.requested_at.unwrap_or_else(Instant::now);
        self.test_mode_active = false;
        self.expected_virtual_keys.clear();

        // Blocks while the keys are typed (~0.4 s); the injected events are
        // read from the terminal afterwards, so the wait starts once it returns.
        match self.sender.send_test_sequence() {
            Ok(keys) => {
                self.begin_check(requested_at, keys, Instant::now());
                Ok(())
            }
            Err(e) => {
                self.diagnostic = DiagnosticResult::ApiIssue;
                Err(e)
            }
        }
    }

    /// Start waiting for the injected `keys`, whose sending finished at `sent_at`.
    fn begin_check(&mut self, requested_at: Instant, keys: Vec<char>, sent_at: Instant) {
        self.test_mode_active = true;
        self.requested_at = Some(requested_at);
        self.virtual_send_time = Some(sent_at);
        self.virtual_keys_received = 0;
        self.test_results.clear();
        for &key in &keys {
            self.test_results.insert(
                key,
                TestKeyResult {
                    virtual_sent: true,
                    ..TestKeyResult::default()
                },
            );
        }
        self.expected_virtual_keys = keys;
    }

    /// Settle the physical-vs-virtual verdict once the wait for the injected
    /// keys is over. Call regularly (the app does so every frame); no further
    /// key press is needed.
    pub fn tick(&mut self) {
        self.tick_at(Instant::now());
    }

    fn tick_at(&mut self, now: Instant) {
        if !self.test_mode_active {
            return;
        }
        let Some(sent_at) = self.virtual_send_time else {
            return;
        };
        if now.saturating_duration_since(sent_at) < INJECTION_WINDOW {
            return; // Still waiting for the injected keys
        }
        let physical_seen = match (self.last_physical_press, self.requested_at) {
            (Some(pressed), Some(requested)) => pressed + TRIGGER_LOOKBACK >= requested,
            _ => false,
        };
        self.diagnostic = diagnose(physical_seen, self.virtual_keys_received > 0);
        self.test_mode_active = false;
    }

    /// Flag the same key re-pressed faster than a finger can.
    fn check_same_key_speed(&mut self, key: KeyCode, timestamp: Instant) -> bool {
        let previous = self.last_press_by_key.insert(key, timestamp);
        let Some(previous) = previous else {
            return false;
        };
        let interval_ms = timestamp.saturating_duration_since(previous).as_secs_f64() * 1000.0;
        if interval_ms < MIN_HUMAN_INTERVAL_MS {
            self.record_anomaly(
                format!("Inhuman speed: same key again after {:.1}ms", interval_ms),
                timestamp,
                AnomalySeverity::High,
            );
            return true;
        }
        false
    }

    /// Analyze the regularity of separate keystrokes (presses made while no
    /// other key was held).
    fn analyze_timing(&mut self, interval_ms: f64, timestamp: Instant) -> bool {
        let mut is_suspicious = false;

        self.intervals.push_back(interval_ms);
        if self.intervals.len() > ANALYSIS_WINDOW {
            self.intervals.pop_front();
        }

        if self.intervals.len() >= 5 {
            let mean: f64 = self.intervals.iter().sum::<f64>() / self.intervals.len() as f64;
            let variance: f64 = self
                .intervals
                .iter()
                .map(|x| (x - mean).powi(2))
                .sum::<f64>()
                / self.intervals.len() as f64;

            self.interval_mean = mean;
            self.interval_variance = variance;

            if variance < PERFECT_TIMING_THRESHOLD
                && self.intervals.len() >= PERFECT_TIMING_MIN_SAMPLES
            {
                self.record_anomaly(
                    format!("Perfect timing: variance={:.2}ms²", variance),
                    timestamp,
                    AnomalySeverity::Medium,
                );
                is_suspicious = true;
            }
        }

        is_suspicious
    }

    /// Detect burst input patterns
    fn detect_burst(&mut self, timestamp: Instant) -> bool {
        let window = Duration::from_millis(BURST_WINDOW_MS);

        self.burst_window.push_back(timestamp);

        while let Some(front) = self.burst_window.front() {
            if timestamp.duration_since(*front) > window {
                self.burst_window.pop_front();
            } else {
                break;
            }
        }

        if self.burst_window.len() >= BURST_COUNT_THRESHOLD {
            self.record_anomaly(
                format!(
                    "{} keys in {}ms window",
                    self.burst_window.len(),
                    BURST_WINDOW_MS
                ),
                timestamp,
                AnomalySeverity::High,
            );
            return true;
        }

        false
    }

    /// Record an anomaly
    fn record_anomaly(
        &mut self,
        description: String,
        timestamp: Instant,
        severity: AnomalySeverity,
    ) {
        if let Some(last) = self.anomalies.back() {
            if timestamp.duration_since(last.timestamp) < Duration::from_millis(100) {
                return;
            }
        }

        self.anomalies.push_back(AnomalyEvent {
            description,
            timestamp,
            severity,
        });

        if self.anomalies.len() > 50 {
            self.anomalies.pop_front();
        }
    }

    /// Update overall session classification
    fn update_classification(&mut self) {
        if self.total_keystrokes < 10 {
            self.session_classification = InputClassification::Uncertain;
            return;
        }

        let virtual_ratio = self.virtual_count as f64 / self.total_keystrokes as f64;
        let recent_anomalies = self
            .anomalies
            .iter()
            .filter(|a| a.timestamp.elapsed() < Duration::from_secs(5))
            .count();

        self.session_classification = if virtual_ratio > 0.5 || recent_anomalies >= 3 {
            InputClassification::Virtual
        } else if virtual_ratio > 0.2 || recent_anomalies >= 1 {
            InputClassification::LikelyVirtual
        } else if virtual_ratio > 0.05 {
            InputClassification::Uncertain
        } else if self.total_keystrokes > 50 {
            InputClassification::Physical
        } else {
            InputClassification::LikelyPhysical
        };
    }

    /// Check if a key matches our expected virtual keys
    fn check_virtual_key_received(&mut self, key: KeyCode) {
        // Exact match on the key's character: "Ctrl" or "Caps" is not 'c'
        let Some(c) = key_char(key) else {
            return;
        };
        if self.expected_virtual_keys.contains(&c) {
            if let Some(result) = self.test_results.get_mut(&c) {
                result.virtual_received = true;
                result._last_test_time = Some(Instant::now());
            }
            self.virtual_keys_received += 1;
        }
    }

    /// Which of the injected keys arrived, e.g. "z x arrived, c missing".
    fn injected_summary(&self) -> Option<String> {
        let sent: Vec<char> = self
            .expected_virtual_keys
            .iter()
            .copied()
            .filter(|c| self.test_results.get(c).is_some_and(|r| r.virtual_sent))
            .collect();
        if sent.is_empty() {
            return None;
        }
        let (arrived, missing): (Vec<char>, Vec<char>) = sent.iter().partition(|c| {
            self.test_results
                .get(*c)
                .is_some_and(|r| r.virtual_received)
        });
        let join = |keys: &[char]| {
            keys.iter()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        };
        Some(match (arrived.is_empty(), missing.is_empty()) {
            (_, true) => format!("{} arrived", join(&arrived)),
            (true, false) => format!("{} missing", join(&missing)),
            (false, false) => format!("{} arrived, {} missing", join(&arrived), join(&missing)),
        })
    }

    /// Current physical-vs-virtual diagnostic result
    pub fn diagnostic(&self) -> DiagnosticResult {
        self.diagnostic
    }

    /// Current input classification (physical vs. virtual heuristics)
    pub fn classification(&self) -> InputClassification {
        self.session_classification
    }

    /// Get recent anomalies
    pub fn recent_anomalies(&self, count: usize) -> Vec<&AnomalyEvent> {
        self.anomalies.iter().rev().take(count).collect()
    }
}

impl Default for VirtualKeyboardTest {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyboardTest for VirtualKeyboardTest {
    fn name(&self) -> &'static str {
        "Virtual Input Detection"
    }

    fn description(&self) -> &'static str {
        "Detects and tests virtual vs physical keyboard input"
    }

    fn process_event(&mut self, event: &KeyEvent) {
        if event.event_type != KeyEventType::Press {
            self.held.remove(&event.key);
            return;
        }

        if self.start_time.is_none() {
            self.start_time = Some(Instant::now());
        }

        self.total_keystrokes += 1;

        // Physical-vs-virtual check: injected keys are never physical evidence
        if is_injected_key(event.key) {
            if self.test_mode_active {
                self.check_virtual_key_received(event.key);
            }
        } else {
            self.last_physical_press = Some(event.timestamp);
        }

        // A press while another key is held is a chord or rolled typing, which
        // people do all the time; only separate keystrokes are timed.
        let chord = self.held.iter().any(|k| *k != event.key);
        if !keymap::is_modifier(event.key) {
            self.held.insert(event.key);
        }

        let interval_ms = self
            .last_keystroke
            .map(|last| event.timestamp.duration_since(last).as_secs_f64() * 1000.0);

        self.last_interval_ms = interval_ms;

        let mut is_virtual = self.check_same_key_speed(event.key, event.timestamp);

        if !chord {
            if let Some(interval) = interval_ms {
                if self.analyze_timing(interval, event.timestamp) {
                    is_virtual = true;
                }
            }

            if self.detect_burst(event.timestamp) {
                is_virtual = true;
            }
        }

        if is_virtual {
            self.virtual_count += 1;
        }

        let record = KeystrokeRecord {
            _key: event.key,
            _timestamp: event.timestamp,
            _interval_ms: interval_ms,
            _is_virtual: is_virtual,
        };
        self.recent_keystrokes.push_back(record);
        if self.recent_keystrokes.len() > ANALYSIS_WINDOW {
            self.recent_keystrokes.pop_front();
        }

        self.last_keystroke = Some(event.timestamp);
        self.update_classification();
    }

    fn is_complete(&self) -> bool {
        false
    }

    fn get_results(&self) -> Vec<TestResult> {
        let mut results = vec![
            // Tooltip: Explain what this test measures
            TestResult::info("--- What This Measures ---", ""),
            TestResult::info("Detects virtual/automated", "input vs physical keys"),
            TestResult::info("'v' injects z, x, c and", "checks they reach the app"),
            TestResult::info(
                "Look for: 'Physical' input,",
                "low suspicious %, no anomalies",
            ),
            TestResult::info("", ""),
            // Diagnostic section
            TestResult::info("=== DIAGNOSTIC TEST ===", ""),
        ];

        if !self.injected_keys_visible {
            results.push(TestResult::info("Status", "Not available with evdev input"));
            results.push(TestResult::info(
                "Info",
                "Injected keys bypass the keyboard device evdev reads",
            ));
        } else {
            results.push(TestResult::new(
                "Status",
                self.diagnostic.as_str(),
                self.diagnostic.to_status(),
            ));
            results.push(TestResult::info(
                "Info",
                self.diagnostic.description().to_string(),
            ));
            if self.diagnostic == DiagnosticResult::NotTested {
                results.push(TestResult::info(
                    "Action",
                    "Press 'v' to send virtual test keys".to_string(),
                ));
            }
            if self.test_mode_active {
                results.push(TestResult::info(
                    "Test Active",
                    "Waiting for the injected keys...".to_string(),
                ));
            }
            if let Some(summary) = self.injected_summary().filter(|_| !self.test_mode_active) {
                results.push(TestResult::info("Injected Keys", summary));
            }
        }
        results.push(TestResult::info(
            "Note",
            "Single keys are verified by the Auto test sweep",
        ));

        // Detection section
        results.push(TestResult::info("=== INPUT DETECTION ===", ""));
        results.push(TestResult::new(
            "Classification",
            self.session_classification.as_str(),
            self.session_classification.to_status(),
        ));

        results.push(TestResult::info(
            "Total Keystrokes",
            format!("{}", self.total_keystrokes),
        ));

        if self.total_keystrokes > 0 {
            let virtual_pct = (self.virtual_count as f64 / self.total_keystrokes as f64) * 100.0;
            let status = if virtual_pct > 20.0 {
                ResultStatus::Warning
            } else if virtual_pct > 5.0 {
                ResultStatus::Info
            } else {
                ResultStatus::Ok
            };
            results.push(TestResult::new(
                "Suspicious Events",
                format!("{} ({:.1}%)", self.virtual_count, virtual_pct),
                status,
            ));
        }

        if let Some(interval) = self.last_interval_ms {
            results.push(TestResult::info(
                "Last Interval",
                format!("{:.1}ms", interval),
            ));
        }

        if self.intervals.len() >= 5 {
            results.push(TestResult::info(
                "Avg Interval",
                format!("{:.1}ms", self.interval_mean),
            ));

            let variance_status = if self.interval_variance < PERFECT_TIMING_THRESHOLD
                && self.intervals.len() >= PERFECT_TIMING_MIN_SAMPLES
            {
                ResultStatus::Warning
            } else {
                ResultStatus::Ok
            };
            results.push(TestResult::new(
                "Timing Variance",
                format!("{:.2}ms²", self.interval_variance),
                variance_status,
            ));
        }

        // Recent anomalies
        let recent = self.recent_anomalies(3);
        if !recent.is_empty() {
            results.push(TestResult::info("--- Anomalies ---", ""));
            for anomaly in recent {
                let status = match anomaly.severity {
                    AnomalySeverity::High => ResultStatus::Error,
                    AnomalySeverity::Medium => ResultStatus::Warning,
                    AnomalySeverity::Low => ResultStatus::Info,
                };
                results.push(TestResult::new("  ", &anomaly.description, status));
            }
        }

        results
    }

    fn reset(&mut self) {
        self.recent_keystrokes.clear();
        self.last_keystroke = None;
        self.total_keystrokes = 0;
        self.virtual_count = 0;
        self.anomalies.clear();
        self.session_classification = InputClassification::Uncertain;
        self.intervals.clear();
        self.burst_window.clear();
        self.start_time = None;
        self.last_interval_ms = None;
        self.interval_variance = 0.0;
        self.interval_mean = 0.0;
        self.held.clear();
        self.last_press_by_key.clear();
        self.expected_virtual_keys.clear();
        self.virtual_send_time = None;
        self.requested_at = None;
        self.last_physical_press = None;
        self.test_results.clear();
        self.diagnostic = self.idle_diagnostic();
        self.virtual_keys_received = 0;
        self.test_mode_active = false;
        self.pending_send = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::test_helpers::{press_at, release_at};

    const KEY_V: KeyCode = KeyCode(47);
    const KEY_Z: KeyCode = KeyCode(44);
    const KEY_X: KeyCode = KeyCode(45);
    const KEY_C: KeyCode = KeyCode(46);
    const KEY_LCTRL: KeyCode = KeyCode(29);
    const KEY_LSHIFT: KeyCode = KeyCode(42);
    const KEY_CAPS: KeyCode = KeyCode(58);

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn tap(test: &mut VirtualKeyboardTest, key: KeyCode, at: Instant, hold: Duration) {
        test.process_event(&press_at(key, at, 0));
        test.process_event(&release_at(key, at + hold));
    }

    fn is_flagged_virtual(test: &VirtualKeyboardTest) -> bool {
        matches!(
            test.classification(),
            InputClassification::Virtual | InputClassification::LikelyVirtual
        )
    }

    /// Start a check as the app does: 'v' requested at `t0`, the blocking
    /// send of z, x, c finishes 400 ms later.
    fn start_check(test: &mut VirtualKeyboardTest, t0: Instant) -> Instant {
        let sent_at = t0 + ms(400);
        test.begin_check(t0, INJECTED_KEYS.to_vec(), sent_at);
        sent_at
    }

    // --- Inhuman-speed heuristics (K-A12) ---------------------------------

    #[test]
    fn chords_rollover_and_shift_are_not_virtual() {
        let mut test = VirtualKeyboardTest::new();
        let mut at = Instant::now();
        // Two-key chords, keys landing 3 ms apart
        for i in 0..12u16 {
            let a = KeyCode(30 + i % 4);
            let b = KeyCode(36 + i % 3);
            test.process_event(&press_at(a, at, 0));
            test.process_event(&press_at(b, at + ms(3), 0));
            test.process_event(&release_at(a, at + ms(90)));
            test.process_event(&release_at(b, at + ms(95)));
            at += ms(250 + u64::from(i) * 17);
        }
        // Shift+letter and Ctrl+C typed quickly
        for (modifier, key) in [(KEY_LSHIFT, KeyCode(35)), (KEY_LCTRL, KEY_C)] {
            test.process_event(&press_at(modifier, at, 0));
            tap(&mut test, key, at + ms(4), ms(60));
            test.process_event(&release_at(modifier, at + ms(80)));
            at += ms(300);
        }
        // NKRO view: slam 8 keys at once (same USB report or within ms)
        for (i, key) in (16u16..24).enumerate() {
            test.process_event(&press_at(KeyCode(key), at + ms(i as u64 / 3), 0));
        }
        for key in 16u16..24 {
            test.process_event(&release_at(KeyCode(key), at + ms(900)));
        }

        assert!(test.total_keystrokes >= 30);
        assert!(test.anomalies.is_empty(), "{:?}", test.anomalies);
        assert_eq!(test.virtual_count, 0);
        assert!(!is_flagged_virtual(&test), "{:?}", test.classification());
    }

    #[test]
    fn irregular_human_typing_is_physical() {
        let mut test = VirtualKeyboardTest::new();
        let mut at = Instant::now();
        for i in 0..60u64 {
            let key = KeyCode(16 + (i * 7 % 20) as u16);
            tap(&mut test, key, at, ms(50 + i * 13 % 40));
            at += ms(90 + i * 37 % 160);
        }
        assert!(test.anomalies.is_empty(), "{:?}", test.anomalies);
        assert_eq!(test.classification(), InputClassification::Physical);
    }

    #[test]
    fn same_key_faster_than_a_finger_is_flagged() {
        let mut test = VirtualKeyboardTest::new();
        let t0 = Instant::now();
        tap(&mut test, KeyCode(30), t0, ms(2));
        tap(&mut test, KeyCode(30), t0 + ms(5), ms(2));
        assert_eq!(test.virtual_count, 1);
        assert!(test.anomalies[0].description.contains("Inhuman speed"));
    }

    #[test]
    fn machine_regular_typing_is_flagged() {
        // Auto-type: every key pressed and released, a new key every 12.0 ms
        let mut test = VirtualKeyboardTest::new();
        let t0 = Instant::now();
        for i in 0..20u64 {
            tap(
                &mut test,
                KeyCode(16 + (i % 10) as u16),
                t0 + ms(12 * i),
                ms(3),
            );
        }
        assert!(test
            .anomalies
            .iter()
            .any(|a| a.description.contains("Perfect timing")));
        assert!(is_flagged_virtual(&test), "{:?}", test.classification());
    }

    // --- Physical vs virtual check (K-A7) --------------------------------

    #[test]
    fn injected_keys_alone_are_not_physical_evidence() {
        let mut test = VirtualKeyboardTest::new();
        let t0 = Instant::now();
        let sent_at = start_check(&mut test, t0);
        for (i, key) in [KEY_Z, KEY_X, KEY_C].into_iter().enumerate() {
            tap(&mut test, key, sent_at + ms(10 + i as u64), ms(20));
        }
        test.tick_at(sent_at + INJECTION_WINDOW);
        // Used to be "Keyboard OK" because z, x, c counted as physical presses
        assert_eq!(test.diagnostic(), DiagnosticResult::Inconclusive);
    }

    #[test]
    fn trigger_press_and_injected_keys_give_ok_without_another_key() {
        let t0 = Instant::now();
        // Terminal order: the 'v' press is processed after the send ...
        let mut late = VirtualKeyboardTest::new();
        let sent_at = start_check(&mut late, t0);
        late.process_event(&press_at(KEY_V, t0 - ms(1), 0));
        // ... evdev-like order: processed before the check started
        let mut early = VirtualKeyboardTest::new();
        early.process_event(&press_at(KEY_V, t0 - ms(5), 0));
        start_check(&mut early, t0);

        for test in [&mut late, &mut early] {
            for (i, key) in [KEY_Z, KEY_X, KEY_C].into_iter().enumerate() {
                tap(test, key, sent_at + ms(10 + i as u64), ms(20));
            }
            // Still waiting: no verdict yet, and none needs another key press
            test.tick_at(sent_at + ms(100));
            assert!(test.test_mode_active);
            assert_eq!(test.diagnostic(), test.idle_diagnostic());
            test.tick_at(sent_at + INJECTION_WINDOW);
            assert!(!test.test_mode_active);
            assert_eq!(test.diagnostic(), DiagnosticResult::KeyboardOk);
            assert!(test
                .get_results()
                .iter()
                .any(|r| r.label == "Injected Keys" && r.value == "z x c arrived"));
        }
    }

    #[test]
    fn blocked_injection_is_an_api_issue_and_ctrl_is_not_c() {
        let mut test = VirtualKeyboardTest::new();
        let t0 = Instant::now();
        let sent_at = start_check(&mut test, t0);
        test.process_event(&press_at(KEY_V, t0, 0));
        // The user fiddles with Ctrl / Caps Lock while waiting
        tap(&mut test, KEY_LCTRL, sent_at + ms(50), ms(40));
        tap(&mut test, KEY_CAPS, sent_at + ms(150), ms(40));
        assert_eq!(test.virtual_keys_received, 0);
        test.tick_at(sent_at + INJECTION_WINDOW);
        assert_eq!(test.diagnostic(), DiagnosticResult::ApiIssue);
        assert!(test
            .get_results()
            .iter()
            .any(|r| r.label == "Injected Keys" && r.value == "z x c missing"));
    }

    #[test]
    fn no_key_reaching_the_test_is_inconclusive_not_a_fault() {
        // App paused, or the start key never reached the test: no events at
        // all, and an old key press from long before the request.
        let mut test = VirtualKeyboardTest::new();
        let t0 = Instant::now() + ms(5000);
        test.process_event(&press_at(KeyCode(30), t0 - ms(3000), 0));
        let sent_at = start_check(&mut test, t0);
        test.tick_at(sent_at + INJECTION_WINDOW);
        let verdict = test.diagnostic();
        assert_eq!(verdict, DiagnosticResult::Inconclusive);
        assert_eq!(verdict.to_status(), ResultStatus::Info);
    }

    #[test]
    fn evdev_input_makes_the_check_unavailable() {
        let mut test = VirtualKeyboardTest::new();
        test.set_input_source(InputSource::Evdev);
        assert_eq!(test.diagnostic(), DiagnosticResult::NotAvailable);
        test.request_virtual_test();
        assert!(!test.has_pending_send());
        let results = test.get_results();
        assert!(results
            .iter()
            .any(|r| r.label == "Status" && r.value.contains("evdev")));
        assert!(!results.iter().any(|r| r.status == ResultStatus::Warning));
        // Still unavailable after a reset
        test.reset();
        assert_eq!(test.diagnostic(), DiagnosticResult::NotAvailable);
    }

    #[test]
    fn diagnose_never_guesses_hardware_or_software_faults() {
        assert_eq!(diagnose(true, true), DiagnosticResult::KeyboardOk);
        assert_eq!(diagnose(true, false), DiagnosticResult::ApiIssue);
        assert_eq!(diagnose(false, true), DiagnosticResult::Inconclusive);
        assert_eq!(diagnose(false, false), DiagnosticResult::Inconclusive);
    }

    #[test]
    fn key_char_matches_single_character_keys_only() {
        assert_eq!(key_char(KEY_C), Some('c'));
        assert_eq!(key_char(KEY_LCTRL), None);
        assert_eq!(key_char(KEY_CAPS), None);
        assert!(is_injected_key(KEY_Z));
        assert!(!is_injected_key(KEY_V));
    }
}
