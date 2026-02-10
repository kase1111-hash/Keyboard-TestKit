//! Virtual keyboard detection and testing module
//!
//! Provides two capabilities:
//! 1. Detection of software-generated keystrokes vs physical input
//! 2. Sending synthetic key events to test software/hardware issues
//!
//! Note: Virtual key sending requires the 'virtual-send' feature and
//! appropriate system libraries (libxdo on Linux, etc.)

use super::{KeyboardTest, ResultStatus, TestResult};
use crate::keyboard::{keymap, KeyCode, KeyEvent, KeyEventType};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

/// Thresholds for virtual input detection
const MIN_HUMAN_INTERVAL_MS: f64 = 15.0;
const PERFECT_TIMING_THRESHOLD: f64 = 0.5;
const BURST_WINDOW_MS: u64 = 50;
const BURST_COUNT_THRESHOLD: usize = 5;
const ANALYSIS_WINDOW: usize = 20;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticResult {
    NotTested,
    NotAvailable,  // Virtual sending not compiled in
    KeyboardOk,    // Physical works + Virtual works
    HardwareIssue, // Physical fails + Virtual works
    SoftwareIssue, // Physical fails + Virtual fails
    ApiIssue,      // Physical works + Virtual fails
}

impl DiagnosticResult {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotTested => "Not tested yet",
            Self::NotAvailable => "Not available",
            Self::KeyboardOk => "Keyboard OK",
            Self::HardwareIssue => "Hardware Issue Detected",
            Self::SoftwareIssue => "Software/Driver Issue",
            Self::ApiIssue => "API/Permission Issue",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::NotTested => "Press 'v' to send virtual test keys",
            Self::NotAvailable => "Build with --features virtual-send for this test",
            Self::KeyboardOk => "Physical and virtual keys both work correctly",
            Self::HardwareIssue => "Physical keys fail but virtual works - check keyboard hardware",
            Self::SoftwareIssue => "Both physical and virtual fail - check drivers/software",
            Self::ApiIssue => "Physical works but virtual fails - check permissions",
        }
    }

    pub fn to_status(self) -> ResultStatus {
        match self {
            Self::NotTested | Self::NotAvailable => ResultStatus::Info,
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

    // Virtual key testing
    /// Virtual key sender
    sender: VirtualKeySender,
    /// Keys we're expecting from virtual send
    expected_virtual_keys: Vec<char>,
    /// Time when we sent virtual keys
    virtual_send_time: Option<Instant>,
    /// Results of key comparison tests
    test_results: HashMap<char, TestKeyResult>,
    /// Current diagnostic result
    diagnostic: DiagnosticResult,
    /// Physical keys received during test
    physical_keys_received: u32,
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
            sender: VirtualKeySender::new(),
            expected_virtual_keys: Vec::new(),
            virtual_send_time: None,
            test_results: HashMap::new(),
            diagnostic,
            physical_keys_received: 0,
            virtual_keys_received: 0,
            test_mode_active: false,
            pending_send: false,
        }
    }

    /// Request to send virtual test keys (called from main loop)
    pub fn request_virtual_test(&mut self) {
        if VirtualKeySender::is_available() {
            self.pending_send = true;
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

        self.test_mode_active = true;
        self.expected_virtual_keys.clear();
        self.virtual_send_time = Some(Instant::now());
        self.physical_keys_received = 0;
        self.virtual_keys_received = 0;

        // Initialize test results for test keys
        for key in ['z', 'x', 'c'] {
            self.test_results.entry(key).or_default().virtual_sent = true;
            self.test_results.entry(key).or_default().virtual_received = false;
        }

        match self.sender.send_test_sequence() {
            Ok(keys) => {
                self.expected_virtual_keys = keys;
                Ok(())
            }
            Err(e) => {
                self.diagnostic = DiagnosticResult::ApiIssue;
                Err(e)
            }
        }
    }

    /// Analyze timing characteristics
    fn analyze_timing(&mut self, interval_ms: f64, timestamp: Instant) -> bool {
        let mut is_suspicious = false;

        if interval_ms < MIN_HUMAN_INTERVAL_MS {
            self.record_anomaly(
                format!("Inhuman speed: {:.1}ms interval", interval_ms),
                timestamp,
                AnomalySeverity::High,
            );
            is_suspicious = true;
        }

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

            if variance < PERFECT_TIMING_THRESHOLD && self.intervals.len() >= 10 {
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
        let key_info = keymap::get_key_info(key);
        let key_char = key_info.label.to_lowercase().chars().next();

        if let Some(c) = key_char {
            if self.expected_virtual_keys.contains(&c) {
                if let Some(result) = self.test_results.get_mut(&c) {
                    result.virtual_received = true;
                    result._last_test_time = Some(Instant::now());
                }
                self.virtual_keys_received += 1;
            }
        }
    }

    /// Update diagnostic based on test results
    fn update_diagnostic(&mut self) {
        if !self.test_mode_active {
            return;
        }

        // Check if enough time has passed since sending (500ms window)
        if let Some(send_time) = self.virtual_send_time {
            if send_time.elapsed() < Duration::from_millis(500) {
                return; // Still waiting for results
            }
        }

        let virtual_worked = self.virtual_keys_received > 0;
        let physical_worked = self.physical_keys_received > 0;

        self.diagnostic = match (physical_worked, virtual_worked) {
            (true, true) => DiagnosticResult::KeyboardOk,
            (false, true) => DiagnosticResult::HardwareIssue,
            (false, false) => DiagnosticResult::SoftwareIssue,
            (true, false) => DiagnosticResult::ApiIssue,
        };

        self.test_mode_active = false;
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
            return;
        }

        if self.start_time.is_none() {
            self.start_time = Some(Instant::now());
        }

        self.total_keystrokes += 1;

        // Track physical keys during test mode
        if self.test_mode_active {
            self.physical_keys_received += 1;
            self.check_virtual_key_received(event.key);
            self.update_diagnostic();
        }

        let interval_ms = self
            .last_keystroke
            .map(|last| event.timestamp.duration_since(last).as_secs_f64() * 1000.0);

        self.last_interval_ms = interval_ms;

        let mut is_virtual = false;

        if let Some(interval) = interval_ms {
            if self.analyze_timing(interval, event.timestamp) {
                is_virtual = true;
            }
        }

        if self.detect_burst(event.timestamp) {
            is_virtual = true;
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
            TestResult::info("Diagnoses hardware vs", "software keyboard issues"),
            TestResult::info(
                "Look for: 'Physical' input,",
                "low suspicious %, no anomalies",
            ),
            TestResult::info("", ""),
            // Diagnostic section
            TestResult::info("=== DIAGNOSTIC TEST ===", ""),
            TestResult::new(
                "Status",
                self.diagnostic.as_str(),
                self.diagnostic.to_status(),
            ),
            TestResult::info("Info", self.diagnostic.description().to_string()),
        ];

        if self.diagnostic == DiagnosticResult::NotTested {
            results.push(TestResult::info(
                "Action",
                "Press 'v' to send virtual test keys".to_string(),
            ));
        }

        if self.test_mode_active {
            results.push(TestResult::warning(
                "Test Active",
                "Waiting for key events...".to_string(),
            ));
        }

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

            let variance_status = if self.interval_variance < PERFECT_TIMING_THRESHOLD {
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
        self.expected_virtual_keys.clear();
        self.virtual_send_time = None;
        self.test_results.clear();
        self.diagnostic = if VirtualKeySender::is_available() {
            DiagnosticResult::NotTested
        } else {
            DiagnosticResult::NotAvailable
        };
        self.physical_keys_received = 0;
        self.virtual_keys_received = 0;
        self.test_mode_active = false;
        self.pending_send = false;
    }
}
