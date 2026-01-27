//! Session report and export functionality
//!
//! Provides test session reports with multiple export formats.
//!
//! ## Supported Formats
//!
//! | Format | Method | Description |
//! |--------|--------|-------------|
//! | JSON | [`SessionReport::export_json`] | Machine-readable structured data |
//! | CSV | [`SessionReport::export_csv`] | Spreadsheet-compatible tabular data |
//! | Markdown | [`SessionReport::export_markdown`] | Human-readable formatted report |
//! | Text | [`SessionReport::export_text`] | Plain text summary |
//!
//! ## Example
//!
//! ```no_run
//! use keyboard_testkit::SessionReport;
//! use std::path::Path;
//!
//! // Assuming you have a report...
//! // report.export_json(Path::new("report.json"))?;
//! // report.export_csv(Path::new("report.csv"))?;
//! // report.export_markdown(Path::new("report.md"))?;
//! // report.export_text(Path::new("report.txt"))?;
//! ```

use crate::keyboard::KeyboardState;
use crate::tests::{TestResult, ResultStatus};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt::Write as FmtWrite;
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
    #[allow(clippy::too_many_arguments)]
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

    // ========================================================================
    // CSV Export
    // ========================================================================

    /// Export report to CSV file
    ///
    /// Creates a CSV with columns: Category, Label, Value, Status
    pub fn export_csv(&self, path: &Path) -> std::io::Result<()> {
        let csv = self.to_csv();
        let mut file = File::create(path)?;
        file.write_all(csv.as_bytes())?;
        Ok(())
    }

    /// Export report to CSV string
    pub fn to_csv(&self) -> String {
        let mut csv = String::new();

        // Header
        writeln!(csv, "Category,Label,Value,Status").unwrap();

        // Helper to write results
        let write_results = |csv: &mut String, category: &str, results: &[ResultEntry]| {
            for entry in results {
                // Escape values that might contain commas or quotes
                let label = Self::csv_escape(&entry.label);
                let value = Self::csv_escape(&entry.value);
                writeln!(csv, "{},{},{},{}", category, label, value, entry.status).unwrap();
            }
        };

        write_results(&mut csv, "Polling", &self.tests.polling);
        write_results(&mut csv, "Hold/Release", &self.tests.hold_release);
        write_results(&mut csv, "Stickiness", &self.tests.stickiness);
        write_results(&mut csv, "Rollover", &self.tests.rollover);
        write_results(&mut csv, "Latency", &self.tests.latency);

        csv
    }

    /// Escape a value for CSV format
    fn csv_escape(value: &str) -> String {
        if value.contains(',') || value.contains('"') || value.contains('\n') {
            format!("\"{}\"", value.replace('"', "\"\""))
        } else {
            value.to_string()
        }
    }

    // ========================================================================
    // Markdown Export
    // ========================================================================

    /// Export report to Markdown file
    pub fn export_markdown(&self, path: &Path) -> std::io::Result<()> {
        let md = self.to_markdown();
        let mut file = File::create(path)?;
        file.write_all(md.as_bytes())?;
        Ok(())
    }

    /// Export report to Markdown string
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();

        // Title
        writeln!(md, "# Keyboard TestKit Report\n").unwrap();

        // Metadata
        writeln!(md, "## Session Information\n").unwrap();
        writeln!(md, "| Property | Value |").unwrap();
        writeln!(md, "|----------|-------|").unwrap();
        writeln!(md, "| Generated | {} |", self.metadata.generated_at).unwrap();
        writeln!(md, "| Version | {} |", self.metadata.version).unwrap();
        writeln!(md, "| Duration | {:.1}s |", self.metadata.duration_secs).unwrap();
        writeln!(md).unwrap();

        // Summary
        writeln!(md, "## Summary\n").unwrap();
        writeln!(md, "| Metric | Value |").unwrap();
        writeln!(md, "|--------|-------|").unwrap();
        writeln!(md, "| Total Events | {} |", self.summary.total_events).unwrap();
        writeln!(md, "| Max Rollover | {}KRO |", self.summary.max_rollover).unwrap();
        if let Some(hz) = self.summary.estimated_polling_rate_hz {
            writeln!(md, "| Polling Rate | {:.0} Hz |", hz).unwrap();
        }
        writeln!(md, "| Issues Detected | {} |", self.summary.issues_detected).unwrap();
        writeln!(md).unwrap();

        // Test results sections
        Self::write_markdown_section(&mut md, "Polling Rate", &self.tests.polling);
        Self::write_markdown_section(&mut md, "Hold/Release", &self.tests.hold_release);
        Self::write_markdown_section(&mut md, "Stickiness", &self.tests.stickiness);
        Self::write_markdown_section(&mut md, "Rollover", &self.tests.rollover);
        Self::write_markdown_section(&mut md, "Latency", &self.tests.latency);

        md
    }

    fn write_markdown_section(md: &mut String, title: &str, results: &[ResultEntry]) {
        if results.is_empty() {
            return;
        }

        writeln!(md, "## {}\n", title).unwrap();
        writeln!(md, "| Metric | Value | Status |").unwrap();
        writeln!(md, "|--------|-------|--------|").unwrap();

        for entry in results {
            let status_emoji = match entry.status.as_str() {
                "ok" => "✅",
                "warning" => "⚠️",
                "error" => "❌",
                _ => "ℹ️",
            };
            writeln!(md, "| {} | {} | {} |", entry.label, entry.value, status_emoji).unwrap();
        }
        writeln!(md).unwrap();
    }

    // ========================================================================
    // Plain Text Export
    // ========================================================================

    /// Export report to plain text file
    pub fn export_text(&self, path: &Path) -> std::io::Result<()> {
        let text = self.to_text();
        let mut file = File::create(path)?;
        file.write_all(text.as_bytes())?;
        Ok(())
    }

    /// Export report to plain text string
    pub fn to_text(&self) -> String {
        let mut text = String::new();

        // Header
        writeln!(text, "KEYBOARD TESTKIT REPORT").unwrap();
        writeln!(text, "=======================\n").unwrap();

        // Metadata
        writeln!(text, "Generated: {}", self.metadata.generated_at).unwrap();
        writeln!(text, "Version:   {}", self.metadata.version).unwrap();
        writeln!(text, "Duration:  {:.1} seconds\n", self.metadata.duration_secs).unwrap();

        // Summary
        writeln!(text, "SUMMARY").unwrap();
        writeln!(text, "-------").unwrap();
        writeln!(text, "Total Events:    {}", self.summary.total_events).unwrap();
        writeln!(text, "Max Rollover:    {}KRO", self.summary.max_rollover).unwrap();
        if let Some(hz) = self.summary.estimated_polling_rate_hz {
            writeln!(text, "Polling Rate:    {:.0} Hz", hz).unwrap();
        }
        writeln!(text, "Issues Detected: {}\n", self.summary.issues_detected).unwrap();

        // Test results
        Self::write_text_section(&mut text, "POLLING RATE", &self.tests.polling);
        Self::write_text_section(&mut text, "HOLD/RELEASE", &self.tests.hold_release);
        Self::write_text_section(&mut text, "STICKINESS", &self.tests.stickiness);
        Self::write_text_section(&mut text, "ROLLOVER", &self.tests.rollover);
        Self::write_text_section(&mut text, "LATENCY", &self.tests.latency);

        text
    }

    fn write_text_section(text: &mut String, title: &str, results: &[ResultEntry]) {
        if results.is_empty() {
            return;
        }

        writeln!(text, "{}", title).unwrap();
        writeln!(text, "{}\n", "-".repeat(title.len())).unwrap();

        for entry in results {
            let status = match entry.status.as_str() {
                "ok" => "[OK]",
                "warning" => "[WARN]",
                "error" => "[ERR]",
                _ => "[INFO]",
            };
            writeln!(text, "  {:<30} {:>20}  {}", entry.label, entry.value, status).unwrap();
        }
        writeln!(text).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_report() -> SessionReport {
        SessionReport {
            metadata: ReportMetadata {
                generated_at: "2024-01-15T12:00:00Z".to_string(),
                version: "0.1.0".to_string(),
                duration_secs: 30.5,
            },
            summary: SessionSummary {
                total_events: 1500,
                max_rollover: 6,
                estimated_polling_rate_hz: Some(1000.0),
                issues_detected: 2,
            },
            tests: TestResults {
                polling: vec![
                    ResultEntry {
                        label: "Average Rate".to_string(),
                        value: "1000 Hz".to_string(),
                        status: "ok".to_string(),
                    },
                    ResultEntry {
                        label: "Jitter".to_string(),
                        value: "50 µs".to_string(),
                        status: "warning".to_string(),
                    },
                ],
                hold_release: vec![],
                stickiness: vec![],
                rollover: vec![
                    ResultEntry {
                        label: "Max Rollover".to_string(),
                        value: "6KRO".to_string(),
                        status: "ok".to_string(),
                    },
                ],
                latency: vec![],
            },
        }
    }

    #[test]
    fn result_entry_from_test_result() {
        let test_result = TestResult::ok("Test Label", "Test Value");
        let entry = ResultEntry::from(&test_result);

        assert_eq!(entry.label, "Test Label");
        assert_eq!(entry.value, "Test Value");
        assert_eq!(entry.status, "ok");
    }

    #[test]
    fn result_entry_status_mapping() {
        let ok = ResultEntry::from(&TestResult::ok("", ""));
        let warn = ResultEntry::from(&TestResult::warning("", ""));
        let err = ResultEntry::from(&TestResult::error("", ""));
        let info = ResultEntry::from(&TestResult::info("", ""));

        assert_eq!(ok.status, "ok");
        assert_eq!(warn.status, "warning");
        assert_eq!(err.status, "error");
        assert_eq!(info.status, "info");
    }

    // JSON tests

    #[test]
    fn to_json_produces_valid_json() {
        let report = create_test_report();
        let json = report.to_json().expect("Failed to serialize");

        assert!(json.contains("\"generated_at\""));
        assert!(json.contains("\"total_events\": 1500"));
        assert!(json.contains("\"max_rollover\": 6"));
        assert!(json.contains("\"Average Rate\""));
    }

    // CSV tests

    #[test]
    fn to_csv_has_header() {
        let report = create_test_report();
        let csv = report.to_csv();

        let first_line = csv.lines().next().unwrap();
        assert_eq!(first_line, "Category,Label,Value,Status");
    }

    #[test]
    fn to_csv_includes_all_results() {
        let report = create_test_report();
        let csv = report.to_csv();

        assert!(csv.contains("Polling,Average Rate,1000 Hz,ok"));
        assert!(csv.contains("Polling,Jitter,50 µs,warning"));
        assert!(csv.contains("Rollover,Max Rollover,6KRO,ok"));
    }

    #[test]
    fn csv_escape_handles_commas() {
        let escaped = SessionReport::csv_escape("value, with, commas");
        assert_eq!(escaped, "\"value, with, commas\"");
    }

    #[test]
    fn csv_escape_handles_quotes() {
        let escaped = SessionReport::csv_escape("value with \"quotes\"");
        assert_eq!(escaped, "\"value with \"\"quotes\"\"\"");
    }

    #[test]
    fn csv_escape_no_special_chars() {
        let escaped = SessionReport::csv_escape("normal value");
        assert_eq!(escaped, "normal value");
    }

    // Markdown tests

    #[test]
    fn to_markdown_has_title() {
        let report = create_test_report();
        let md = report.to_markdown();

        assert!(md.starts_with("# Keyboard TestKit Report"));
    }

    #[test]
    fn to_markdown_includes_metadata() {
        let report = create_test_report();
        let md = report.to_markdown();

        assert!(md.contains("## Session Information"));
        assert!(md.contains("2024-01-15T12:00:00Z"));
        assert!(md.contains("0.1.0"));
        assert!(md.contains("30.5s"));
    }

    #[test]
    fn to_markdown_includes_summary() {
        let report = create_test_report();
        let md = report.to_markdown();

        assert!(md.contains("## Summary"));
        assert!(md.contains("1500"));
        assert!(md.contains("6KRO"));
        assert!(md.contains("1000 Hz"));
    }

    #[test]
    fn to_markdown_uses_status_emojis() {
        let report = create_test_report();
        let md = report.to_markdown();

        assert!(md.contains("✅")); // ok status
        assert!(md.contains("⚠️")); // warning status
    }

    // Plain text tests

    #[test]
    fn to_text_has_header() {
        let report = create_test_report();
        let text = report.to_text();

        assert!(text.starts_with("KEYBOARD TESTKIT REPORT"));
    }

    #[test]
    fn to_text_includes_summary() {
        let report = create_test_report();
        let text = report.to_text();

        assert!(text.contains("SUMMARY"));
        assert!(text.contains("Total Events:    1500"));
        assert!(text.contains("Max Rollover:    6KRO"));
    }

    #[test]
    fn to_text_uses_status_tags() {
        let report = create_test_report();
        let text = report.to_text();

        assert!(text.contains("[OK]"));
        assert!(text.contains("[WARN]"));
    }

    #[test]
    fn to_text_includes_sections() {
        let report = create_test_report();
        let text = report.to_text();

        assert!(text.contains("POLLING RATE"));
        assert!(text.contains("ROLLOVER"));
    }
}
