//! Configuration management for Keyboard TestKit

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Main application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Polling rate test settings
    pub polling: PollingConfig,
    /// Stickiness detection settings
    pub stickiness: StickinessConfig,
    /// Hold and release test settings
    pub hold_release: HoldReleaseConfig,
    /// UI settings
    pub ui: UiConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            polling: PollingConfig::default(),
            stickiness: StickinessConfig::default(),
            hold_release: HoldReleaseConfig::default(),
            ui: UiConfig::default(),
        }
    }
}

/// Polling rate test configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollingConfig {
    /// Test duration in seconds
    pub test_duration_secs: u64,
    /// Sample window for averaging (in milliseconds)
    pub sample_window_ms: u64,
}

impl Default for PollingConfig {
    fn default() -> Self {
        Self {
            test_duration_secs: 10,
            sample_window_ms: 100,
        }
    }
}

/// Stickiness detection configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StickinessConfig {
    /// Threshold in ms after which a key is considered stuck
    pub stuck_threshold_ms: u64,
    /// Enable audio alerts for stuck keys
    pub audio_alert: bool,
}

impl Default for StickinessConfig {
    fn default() -> Self {
        Self {
            stuck_threshold_ms: 50,
            audio_alert: false,
        }
    }
}

/// Hold and release test configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HoldReleaseConfig {
    /// Bounce detection window in ms
    pub bounce_window_ms: u64,
    /// Minimum hold time to register as intentional
    pub min_hold_ms: u64,
}

impl Default for HoldReleaseConfig {
    fn default() -> Self {
        Self {
            bounce_window_ms: 5,
            min_hold_ms: 10,
        }
    }
}

/// UI configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    /// Refresh rate for UI updates (in Hz)
    pub refresh_rate_hz: u32,
    /// Show warning overlay duration in seconds
    pub warning_duration_secs: u32,
    /// Color theme (dark/light)
    pub theme: Theme,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            refresh_rate_hz: 60,
            warning_duration_secs: 3,
            theme: Theme::Dark,
        }
    }
}

/// Color theme options
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
}

impl Config {
    /// Get UI refresh interval as Duration
    pub fn refresh_interval(&self) -> Duration {
        Duration::from_micros(1_000_000 / self.ui.refresh_rate_hz as u64)
    }
}
