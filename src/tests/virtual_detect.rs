//! Virtual keyboard detection test module
//!
//! Detects software-generated keystrokes vs physical input by analyzing
//! timing patterns, regularity, and speed characteristics.

use super::{KeyboardTest, TestResult, ResultStatus};
use crate::keyboard::{KeyCode, KeyEvent, KeyEventType, keymap};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Thresholds for virtual input detection
const MIN_HUMAN_INTERVAL_MS: f64 = 15.0;      // Faster than this is likely virtual
const PERFECT_TIMING_THRESHOLD: f64 = 0.5;     // Variance below this is suspicious
const BURST_WINDOW_MS: u64 = 50;               // Window to detect burst input
const BURST_COUNT_THRESHOLD: usize = 5;        // Keys in burst window = suspicious
const ANALYSIS_WINDOW: usize = 20;             // Rolling window for analysis

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

    pub fn to_status(&self) -> ResultStatus {
        match self {
            Self::Physical | Self::LikelyPhysical => ResultStatus::Ok,
            Self::Uncertain => ResultStatus::Info,
            Self::LikelyVirtual | Self::Virtual => ResultStatus::Warning,
        }
    }
}

/// Record of a keystroke for analysis
#[derive(Debug, Clone)]
struct KeystrokeRecord {
    key: KeyCode,
    timestamp: Instant,
    interval_ms: Option<f64>,
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

/// Virtual keyboard detection test
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
    anomalies: Vec<AnomalyEvent>,
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
}

impl VirtualKeyboardTest {
    pub fn new() -> Self {
        Self {
            recent_keystrokes: VecDeque::with_capacity(ANALYSIS_WINDOW + 1),
            last_keystroke: None,
            total_keystrokes: 0,
            virtual_count: 0,
            anomalies: Vec::new(),
            session_classification: InputClassification::Uncertain,
            intervals: VecDeque::with_capacity(ANALYSIS_WINDOW + 1),
            burst_window: VecDeque::new(),
            start_time: None,
            last_interval_ms: None,
            interval_variance: 0.0,
            interval_mean: 0.0,
        }
    }

    /// Analyze timing characteristics
    fn analyze_timing(&mut self, interval_ms: f64, timestamp: Instant) -> bool {
        let mut is_suspicious = false;

        // Check for inhuman speed
        if interval_ms < MIN_HUMAN_INTERVAL_MS {
            self.record_anomaly(
                format!("Inhuman speed: {:.1}ms interval", interval_ms),
                timestamp,
                AnomalySeverity::High,
            );
            is_suspicious = true;
        }

        // Update rolling statistics
        self.intervals.push_back(interval_ms);
        if self.intervals.len() > ANALYSIS_WINDOW {
            self.intervals.pop_front();
        }

        // Calculate variance if we have enough samples
        if self.intervals.len() >= 5 {
            let mean: f64 = self.intervals.iter().sum::<f64>() / self.intervals.len() as f64;
            let variance: f64 = self.intervals.iter()
                .map(|x| (x - mean).powi(2))
                .sum::<f64>() / self.intervals.len() as f64;

            self.interval_mean = mean;
            self.interval_variance = variance;

            // Check for unnaturally consistent timing
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

        // Add current timestamp
        self.burst_window.push_back(timestamp);

        // Remove old timestamps outside window
        while let Some(front) = self.burst_window.front() {
            if timestamp.duration_since(*front) > window {
                self.burst_window.pop_front();
            } else {
                break;
            }
        }

        // Check if we have a burst
        if self.burst_window.len() >= BURST_COUNT_THRESHOLD {
            self.record_anomaly(
                format!("{} keys in {}ms window", self.burst_window.len(), BURST_WINDOW_MS),
                timestamp,
                AnomalySeverity::High,
            );
            return true;
        }

        false
    }

    /// Record an anomaly
    fn record_anomaly(&mut self, description: String, timestamp: Instant, severity: AnomalySeverity) {
        // Avoid duplicate anomalies within 100ms
        if let Some(last) = self.anomalies.last() {
            if timestamp.duration_since(last.timestamp) < Duration::from_millis(100) {
                return;
            }
        }

        self.anomalies.push(AnomalyEvent {
            description,
            timestamp,
            severity,
        });

        // Keep only last 50 anomalies
        if self.anomalies.len() > 50 {
            self.anomalies.remove(0);
        }
    }

    /// Update overall session classification
    fn update_classification(&mut self) {
        if self.total_keystrokes < 10 {
            self.session_classification = InputClassification::Uncertain;
            return;
        }

        let virtual_ratio = self.virtual_count as f64 / self.total_keystrokes as f64;
        let recent_anomalies = self.anomalies.iter()
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

    /// Get key name for display
    fn get_key_name(key: KeyCode) -> String {
        keymap::get_key_info(key).name.to_string()
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
        "Detects software-generated keystrokes vs physical input"
    }

    fn process_event(&mut self, event: &KeyEvent) {
        // Only analyze key presses, not releases
        if event.event_type != KeyEventType::Press {
            return;
        }

        if self.start_time.is_none() {
            self.start_time = Some(Instant::now());
        }

        self.total_keystrokes += 1;

        // Calculate interval from last keystroke
        let interval_ms = self.last_keystroke.map(|last| {
            event.timestamp.duration_since(last).as_secs_f64() * 1000.0
        });

        self.last_interval_ms = interval_ms;

        // Analyze for virtual input indicators
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

        // Record keystroke
        let record = KeystrokeRecord {
            key: event.key,
            timestamp: event.timestamp,
            interval_ms,
        };
        self.recent_keystrokes.push_back(record);
        if self.recent_keystrokes.len() > ANALYSIS_WINDOW {
            self.recent_keystrokes.pop_front();
        }

        self.last_keystroke = Some(event.timestamp);
        self.update_classification();
    }

    fn is_complete(&self) -> bool {
        false // Continuous test
    }

    fn get_results(&self) -> Vec<TestResult> {
        let mut results = Vec::new();

        // Overall classification
        results.push(TestResult::new(
            "Input Classification",
            self.session_classification.as_str(),
            self.session_classification.to_status(),
        ));

        // Statistics
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

        // Timing stats
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

        // Thresholds info
        results.push(TestResult::info("--- Detection Thresholds ---", ""));
        results.push(TestResult::info(
            "Min Human Interval",
            format!("{}ms", MIN_HUMAN_INTERVAL_MS),
        ));
        results.push(TestResult::info(
            "Burst Detection",
            format!("{} keys/{}ms", BURST_COUNT_THRESHOLD, BURST_WINDOW_MS),
        ));

        // Recent anomalies
        let recent = self.recent_anomalies(5);
        if !recent.is_empty() {
            results.push(TestResult::info("--- Recent Anomalies ---", ""));
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
    }
}
