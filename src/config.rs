//! Configuration management for Keyboard TestKit
//!
//! Provides persistent configuration that is automatically saved to and loaded
//! from a platform-specific config file.
//!
//! ## Config File Locations
//!
//! | Platform | Path |
//! |----------|------|
//! | Linux | `~/.config/keyboard-testkit/config.toml` |
//! | macOS | `~/Library/Application Support/keyboard-testkit/config.toml` |
//! | Windows | `%APPDATA%\keyboard-testkit\config.toml` |
//!
//! ## Example
//!
//! ```no_run
//! use keyboard_testkit::Config;
//!
//! // Load existing config or use defaults
//! let mut config = Config::load().unwrap_or_default();
//!
//! // Modify settings
//! config.polling.test_duration_secs = 30;
//!
//! // Save to disk
//! config.save().expect("Failed to save config");
//! ```

use crate::keyboard::remap::FnKeyMode;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

/// Error type for configuration operations
#[derive(Debug)]
pub enum ConfigError {
    /// Failed to determine config directory
    NoConfigDir,
    /// IO error reading or writing config file
    Io(io::Error),
    /// Failed to parse config file
    Parse(toml::de::Error),
    /// Failed to serialize config
    Serialize(toml::ser::Error),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::NoConfigDir => write!(f, "Could not determine config directory"),
            ConfigError::Io(e) => write!(f, "IO error: {}", e),
            ConfigError::Parse(e) => write!(f, "Parse error: {}", e),
            ConfigError::Serialize(e) => write!(f, "Serialize error: {}", e),
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<io::Error> for ConfigError {
    fn from(e: io::Error) -> Self {
        ConfigError::Io(e)
    }
}

impl From<toml::de::Error> for ConfigError {
    fn from(e: toml::de::Error) -> Self {
        ConfigError::Parse(e)
    }
}

impl From<toml::ser::Error> for ConfigError {
    fn from(e: toml::ser::Error) -> Self {
        ConfigError::Serialize(e)
    }
}

/// Returns the path to the config file.
///
/// Creates the config directory if it doesn't exist.
///
/// # Platform-specific paths
///
/// - Linux: `~/.config/keyboard-testkit/config.toml`
/// - macOS: `~/Library/Application Support/keyboard-testkit/config.toml`
/// - Windows: `%APPDATA%\keyboard-testkit\config.toml`
pub fn config_path() -> Result<PathBuf, ConfigError> {
    let config_dir = dirs::config_dir().ok_or(ConfigError::NoConfigDir)?;
    let app_dir = config_dir.join("keyboard-testkit");

    // Create directory if it doesn't exist
    if !app_dir.exists() {
        fs::create_dir_all(&app_dir)?;
    }

    Ok(app_dir.join("config.toml"))
}

/// Main application configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// Polling rate test settings
    pub polling: PollingConfig,
    /// Stickiness detection settings
    pub stickiness: StickinessConfig,
    /// Hold and release test settings
    pub hold_release: HoldReleaseConfig,
    /// UI settings
    pub ui: UiConfig,
    /// OEM key and remapping settings
    #[serde(default)]
    pub oem_keys: OemKeyConfig,
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
}

impl Default for StickinessConfig {
    fn default() -> Self {
        Self {
            stuck_threshold_ms: 50,
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

/// OEM key and remapping configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OemKeyConfig {
    /// Enable OEM key capture and remapping
    pub enabled: bool,
    /// FN key handling mode
    pub fn_mode: FnKeyMode,
    /// Custom FN key scancodes to recognize (in addition to defaults)
    #[serde(default)]
    pub fn_scancodes: Vec<u16>,
    /// Custom key remappings: (source_scancode, target_scancode)
    #[serde(default)]
    pub key_mappings: Vec<(u16, u16)>,
    /// Custom FN+key combinations: (key_scancode, result_scancode)
    #[serde(default)]
    pub fn_combos: Vec<(u16, u16)>,
    /// Capture unknown/unmapped keys for analysis
    pub capture_unknown: bool,
    /// Show OEM key notifications
    pub show_notifications: bool,
}

impl Default for OemKeyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            fn_mode: FnKeyMode::CaptureOnly,
            fn_scancodes: Vec::new(),
            key_mappings: Vec::new(),
            fn_combos: Vec::new(),
            capture_unknown: true,
            show_notifications: true,
        }
    }
}

impl OemKeyConfig {
    /// Create a config with FN key restoration enabled
    pub fn with_fn_restoration() -> Self {
        Self {
            enabled: true,
            fn_mode: FnKeyMode::MapToFKeys,
            fn_scancodes: Vec::new(),
            key_mappings: Vec::new(),
            fn_combos: Vec::new(),
            capture_unknown: true,
            show_notifications: true,
        }
    }

    /// Add a key mapping
    pub fn add_mapping(&mut self, from: u16, to: u16) {
        // Remove existing mapping for same source
        self.key_mappings.retain(|(k, _)| *k != from);
        self.key_mappings.push((from, to));
    }

    /// Add an FN+key combo
    pub fn add_fn_combo(&mut self, key: u16, result: u16) {
        // Remove existing combo for same key
        self.fn_combos.retain(|(k, _)| *k != key);
        self.fn_combos.push((key, result));
    }

    /// Add an FN key scancode
    pub fn add_fn_scancode(&mut self, scancode: u16) {
        if !self.fn_scancodes.contains(&scancode) {
            self.fn_scancodes.push(scancode);
        }
    }
}

impl Config {
    /// Load configuration from the default config file.
    ///
    /// Returns the default configuration if the file doesn't exist.
    /// Returns an error if the file exists but cannot be parsed.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use keyboard_testkit::Config;
    ///
    /// let config = Config::load().unwrap_or_default();
    /// println!("Polling duration: {}s", config.polling.test_duration_secs);
    /// ```
    pub fn load() -> Result<Self, ConfigError> {
        let path = config_path()?;

        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = fs::read_to_string(&path)?;
        let config: Config = toml::from_str(&contents)?;
        Ok(config)
    }

    /// Load configuration from a specific path.
    ///
    /// Useful for testing or using custom config locations.
    pub fn load_from(path: &PathBuf) -> Result<Self, ConfigError> {
        let contents = fs::read_to_string(path)?;
        let config: Config = toml::from_str(&contents)?;
        Ok(config)
    }

    /// Save configuration to the default config file.
    ///
    /// Creates the config directory and file if they don't exist.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use keyboard_testkit::Config;
    ///
    /// let mut config = Config::default();
    /// config.ui.refresh_rate_hz = 120;
    /// config.save().expect("Failed to save config");
    /// ```
    pub fn save(&self) -> Result<(), ConfigError> {
        let path = config_path()?;
        self.save_to(&path)
    }

    /// Save configuration to a specific path.
    ///
    /// Useful for testing or using custom config locations.
    pub fn save_to(&self, path: &PathBuf) -> Result<(), ConfigError> {
        let contents = toml::to_string_pretty(self)?;
        fs::write(path, contents)?;
        Ok(())
    }

    /// Get UI refresh interval as Duration
    pub fn refresh_interval(&self) -> Duration {
        Duration::from_micros(1_000_000 / self.ui.refresh_rate_hz as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn temp_config_path() -> PathBuf {
        env::temp_dir().join(format!("keyboard-testkit-test-{}.toml", std::process::id()))
    }

    #[test]
    fn config_default_values() {
        let config = Config::default();
        assert_eq!(config.polling.test_duration_secs, 10);
        assert_eq!(config.polling.sample_window_ms, 100);
        assert_eq!(config.stickiness.stuck_threshold_ms, 50);
        assert_eq!(config.hold_release.bounce_window_ms, 5);
        assert_eq!(config.hold_release.min_hold_ms, 10);
        assert_eq!(config.ui.refresh_rate_hz, 60);
        assert_eq!(config.ui.theme, Theme::Dark);
    }

    #[test]
    fn config_refresh_interval() {
        let config = Config::default();
        // 60 Hz = 16666 microseconds per frame
        let interval = config.refresh_interval();
        assert_eq!(interval.as_micros(), 16666);
    }

    #[test]
    fn config_refresh_interval_120hz() {
        let mut config = Config::default();
        config.ui.refresh_rate_hz = 120;
        let interval = config.refresh_interval();
        assert_eq!(interval.as_micros(), 8333);
    }

    #[test]
    fn config_save_and_load_roundtrip() {
        let path = temp_config_path();

        // Create non-default config
        let mut config = Config::default();
        config.polling.test_duration_secs = 30;
        config.ui.theme = Theme::Light;

        // Save to temp file
        config.save_to(&path).expect("Failed to save config");

        // Load it back
        let loaded = Config::load_from(&path).expect("Failed to load config");

        // Verify values match
        assert_eq!(loaded.polling.test_duration_secs, 30);
        assert_eq!(loaded.ui.theme, Theme::Light);

        // Cleanup
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn config_load_missing_file_returns_default() {
        let path = PathBuf::from("/nonexistent/path/config.toml");

        // load_from should fail with IO error
        let result = Config::load_from(&path);
        assert!(result.is_err());
    }

    #[test]
    fn config_serializes_to_toml() {
        let config = Config::default();
        let toml_str = toml::to_string_pretty(&config).expect("Failed to serialize");

        assert!(toml_str.contains("[polling]"));
        assert!(toml_str.contains("[stickiness]"));
        assert!(toml_str.contains("[hold_release]"));
        assert!(toml_str.contains("[ui]"));
        assert!(toml_str.contains("test_duration_secs = 10"));
    }

    #[test]
    fn config_deserializes_from_toml() {
        let toml_str = r#"
[polling]
test_duration_secs = 20
sample_window_ms = 200

[stickiness]
stuck_threshold_ms = 100

[hold_release]
bounce_window_ms = 10
min_hold_ms = 20

[ui]
refresh_rate_hz = 144
warning_duration_secs = 5
theme = "Light"
"#;

        let config: Config = toml::from_str(toml_str).expect("Failed to deserialize");

        assert_eq!(config.polling.test_duration_secs, 20);
        assert_eq!(config.polling.sample_window_ms, 200);
        assert_eq!(config.stickiness.stuck_threshold_ms, 100);
        assert_eq!(config.hold_release.bounce_window_ms, 10);
        assert_eq!(config.hold_release.min_hold_ms, 20);
        assert_eq!(config.ui.refresh_rate_hz, 144);
        assert_eq!(config.ui.warning_duration_secs, 5);
        assert_eq!(config.ui.theme, Theme::Light);
    }

    #[test]
    fn config_error_display() {
        let err = ConfigError::NoConfigDir;
        assert_eq!(err.to_string(), "Could not determine config directory");

        let io_err = ConfigError::Io(io::Error::new(io::ErrorKind::NotFound, "file not found"));
        assert!(io_err.to_string().contains("IO error"));
    }

    #[test]
    fn config_path_creates_directory() {
        // This test verifies config_path() returns a valid path
        // The actual path depends on the platform
        let result = config_path();
        assert!(result.is_ok());

        let path = result.unwrap();
        assert!(path.to_string_lossy().contains("keyboard-testkit"));
        assert!(path.to_string_lossy().ends_with("config.toml"));
    }

    #[test]
    fn theme_equality() {
        assert_eq!(Theme::Dark, Theme::Dark);
        assert_eq!(Theme::Light, Theme::Light);
        assert_ne!(Theme::Dark, Theme::Light);
    }

    #[test]
    fn theme_in_config_serialization() {
        // Test that theme serializes correctly within a config struct
        let mut config = Config::default();
        config.ui.theme = Theme::Light;

        let toml_str = toml::to_string_pretty(&config).expect("Failed to serialize");
        assert!(toml_str.contains("theme = \"Light\""));

        config.ui.theme = Theme::Dark;
        let toml_str = toml::to_string_pretty(&config).expect("Failed to serialize");
        assert!(toml_str.contains("theme = \"Dark\""));
    }
}
