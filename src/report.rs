//! Session report and export functionality

use crate::keyboard::KeyboardState;
use crate::tests::{TestResult, ResultStatus};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::time::Instant;

/// Complete session report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionReport {
    /// Report metadata
    pub metadata: ReportMetadata,
    /// Summary statistics
    pub summary: SessionSummary,
    /// Test results by category
    pub tests: TestResults,
}

/// Report metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportMetadata {
    /// Report generation timestamp
    pub generated_at: String,
    /// Application version
    pub version: String,
    /// Session duration in seconds
    pub duration_secs: f64,
}

/// Session summary statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    /// Total keyboard events processed
    pub total_events: u64,
    /// Maximum simultaneous keys pressed
    pub max_rollover: usize,
    /// Estimated polling rate in Hz
    pub estimated_polling_rate_hz: Option<f64>,
    /// Number of issues detected
    pub issues_detected: u32,
}

/// All test results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResults {
    pub polling: Vec<ResultEntry>,
    pub hold_release: Vec<ResultEntry>,
    pub stickiness: Vec<ResultEntry>,
    pub rollover: Vec<ResultEntry>,
    pub latency: Vec<ResultEntry>,
}

/// Single result entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultEntry {
    pub label: String,
    pub value: String,
    pub status: String,
}

impl From<&TestResult> for ResultEntry {
    fn from(result: &TestResult) -> Self {
        let status = match result.status {
            ResultStatus::Ok => "ok",
            ResultStatus::Warning => "warning",
            ResultStatus::Error => "error",
            ResultStatus::Info => "info",
        };
        Self {
            label: result.label.clone(),
            value: result.value.clone(),
            status: status.to_string(),
        }
    }
}

impl SessionReport {
    /// Create a new session report
    pub fn new(
        start_time: Instant,
        total_events: u64,
        keyboard_state: &KeyboardState,
        polling_results: Vec<TestResult>,
        hold_release_results: Vec<TestResult>,
        stickiness_results: Vec<TestResult>,
        rollover_results: Vec<TestResult>,
        latency_results: Vec<TestResult>,
    ) -> Self {
        let duration_secs = start_time.elapsed().as_secs_f64();
        let now: DateTime<Utc> = Utc::now();

        // Count issues (warnings and errors)
        let count_issues = |results: &[TestResult]| -> u32 {
            results.iter().filter(|r| {
                matches!(r.status, ResultStatus::Warning | ResultStatus::Error)
            }).count() as u32
        };

        let issues = count_issues(&polling_results)
            + count_issues(&hold_release_results)
            + count_issues(&stickiness_results)
            + count_issues(&rollover_results)
            + count_issues(&latency_results);

        Self {
            metadata: ReportMetadata {
                generated_at: now.to_rfc3339(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                duration_secs,
            },
            summary: SessionSummary {
                total_events,
                max_rollover: keyboard_state.max_rollover(),
                estimated_polling_rate_hz: keyboard_state.global_polling_rate_hz(),
                issues_detected: issues,
            },
            tests: TestResults {
                polling: polling_results.iter().map(ResultEntry::from).collect(),
                hold_release: hold_release_results.iter().map(ResultEntry::from).collect(),
                stickiness: stickiness_results.iter().map(ResultEntry::from).collect(),
                rollover: rollover_results.iter().map(ResultEntry::from).collect(),
                latency: latency_results.iter().map(ResultEntry::from).collect(),
            },
        }
    }

    /// Export report to JSON file
    pub fn export_json(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let mut file = File::create(path)?;
        file.write_all(json.as_bytes())?;
        Ok(())
    }

    /// Export report to JSON string
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}
