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
    let config_dir = platform_config_dir().ok_or(ConfigError::NoConfigDir)?;
    let app_dir = config_dir.join("keyboard-testkit");

    // Create directory if it doesn't exist
    if !app_dir.exists() {
        fs::create_dir_all(&app_dir)?;
    }

    Ok(app_dir.join("config.toml"))
}

/// Get the platform-specific config directory without the `dirs` crate.
fn platform_config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        // $XDG_CONFIG_HOME or ~/.config
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            if !xdg.is_empty() {
                return Some(PathBuf::from(xdg));
            }
        }
        std::env::var("HOME")
            .ok()
            .map(|h| PathBuf::from(h).join(".config"))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var("HOME")
            .ok()
            .map(|h| PathBuf::from(h).join("Library/Application Support"))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var("APPDATA").ok().map(PathBuf::from)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        std::env::var("HOME")
            .ok()
            .map(|h| PathBuf::from(h).join(".config"))
    }
}

// ----------------------------------------------------------------------------
// Allowed ranges. A hand-edited config.toml is clamped to these on load (see
// `Config::sanitize`); the Settings view stays within them.
// ----------------------------------------------------------------------------

/// Shortest polling run. The Settings view steps in 5 s; a shorter run does
/// not collect enough keystrokes for a rate estimate (0 ends before it starts).
pub const POLLING_DURATION_SECS_MIN: u64 = 5;
/// Longest polling run. Nobody types for an hour to measure a poll rate.
pub const POLLING_DURATION_SECS_MAX: u64 = 3600;
/// Polling sample window: at least one 125 Hz poll interval (8 ms, rounded
/// up to 10 ms)...
pub const SAMPLE_WINDOW_MS_MIN: u64 = 10;
/// ...and at most 10 s (the default polling run); a longer window is in
/// effect the whole-run average.
pub const SAMPLE_WINDOW_MS_MAX: u64 = 10_000;
/// Lowest stuck-key threshold. A keystroke lasts 50-150 ms, so anything
/// shorter would flag ordinary typing; 250 ms is also the Settings view step.
pub const STUCK_THRESHOLD_MS_MIN: u64 = 250;
/// Highest stuck-key threshold. Past a minute the check would practically
/// never fire during a test session.
pub const STUCK_THRESHOLD_MS_MAX: u64 = 60_000;
/// Bounce window: 0 would disable bounce detection entirely...
pub const BOUNCE_WINDOW_MS_MIN: u64 = 1;
/// ...and 100 ms is already about twice the release-to-press gap of a
/// deliberate fast double tap (~50 ms); a wider window would count ordinary
/// typing as bounce.
pub const BOUNCE_WINDOW_MS_MAX: u64 = 100;
/// UI refresh rate. Below 10 Hz the screen and the stuck-key check visibly
/// lag (0 would divide by zero)...
pub const REFRESH_RATE_HZ_MIN: u32 = 10;
/// ...and displays (so terminal emulators) rarely refresh faster than 240 Hz,
/// so a higher rate only burns CPU.
pub const REFRESH_RATE_HZ_MAX: u32 = 240;
/// Shortcut overlay duration: 0 would hide it immediately...
pub const WARNING_DURATION_SECS_MIN: u32 = 1;
/// ...and a huge value would leave it covering the results.
pub const WARNING_DURATION_SECS_MAX: u32 = 30;

/// Main application configuration
///
/// Missing sections or fields fall back to their defaults, so a partly
/// hand-written config.toml still loads.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
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
#[serde(default)]
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
#[serde(default)]
pub struct StickinessConfig {
    /// Threshold in ms after which a key is considered stuck
    pub stuck_threshold_ms: u64,
}

impl Default for StickinessConfig {
    fn default() -> Self {
        Self {
            // A normal key press lasts 50-150 ms and modifiers are routinely
            // held for a second or more, so anything below ~2 s flags normal
            // typing as "stuck".
            stuck_threshold_ms: 2000,
        }
    }
}

/// Hold and release test configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HoldReleaseConfig {
    /// Bounce detection window in ms
    pub bounce_window_ms: u64,
    /// Minimum hold time to register as intentional.
    /// Not used by any test yet; kept so existing config files still load.
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
#[serde(default)]
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
#[serde(default)]
pub struct OemKeyConfig {
    /// Apply the custom `key_mappings` (FN handling is set by `fn_mode`)
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
    /// Capture unknown/unmapped keys for analysis.
    /// Not used yet (unknown keys are always captured); kept so existing
    /// config files still load.
    pub capture_unknown: bool,
    /// Show OEM key notifications.
    /// Not used yet; kept so existing config files still load.
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
            // config dir created
            // debug:("No config file at {}, using defaults", path.display());
            return Ok(Self::default());
        }

        Self::load_from(&path)
    }

    /// Load configuration from a specific path.
    ///
    /// Useful for testing or using custom config locations. Out-of-range
    /// values are clamped (see `sanitize`).
    pub fn load_from(path: &PathBuf) -> Result<Self, ConfigError> {
        let contents = fs::read_to_string(path)?;
        let mut config: Config = toml::from_str(&contents)?;
        config.sanitize();
        Ok(config)
    }

    /// Clamp every value to the range the app can work with, so a
    /// hand-edited config.toml cannot crash the app (refresh rate 0) or
    /// silently break a test (stuck threshold 0 flags every keystroke).
    ///
    /// Returns a description of each value that was changed.
    pub fn sanitize(&mut self) -> Vec<String> {
        let mut changes = Vec::new();
        clamp_field(
            &mut changes,
            "polling.test_duration_secs",
            &mut self.polling.test_duration_secs,
            POLLING_DURATION_SECS_MIN,
            POLLING_DURATION_SECS_MAX,
        );
        clamp_field(
            &mut changes,
            "polling.sample_window_ms",
            &mut self.polling.sample_window_ms,
            SAMPLE_WINDOW_MS_MIN,
            SAMPLE_WINDOW_MS_MAX,
        );
        clamp_field(
            &mut changes,
            "stickiness.stuck_threshold_ms",
            &mut self.stickiness.stuck_threshold_ms,
            STUCK_THRESHOLD_MS_MIN,
            STUCK_THRESHOLD_MS_MAX,
        );
        clamp_field(
            &mut changes,
            "hold_release.bounce_window_ms",
            &mut self.hold_release.bounce_window_ms,
            BOUNCE_WINDOW_MS_MIN,
            BOUNCE_WINDOW_MS_MAX,
        );
        clamp_field(
            &mut changes,
            "ui.refresh_rate_hz",
            &mut self.ui.refresh_rate_hz,
            REFRESH_RATE_HZ_MIN,
            REFRESH_RATE_HZ_MAX,
        );
        clamp_field(
            &mut changes,
            "ui.warning_duration_secs",
            &mut self.ui.warning_duration_secs,
            WARNING_DURATION_SECS_MIN,
            WARNING_DURATION_SECS_MAX,
        );
        changes
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
        fs::write(path, &contents)?;
        Ok(())
    }

    /// Get UI refresh interval as Duration
    pub fn refresh_interval(&self) -> Duration {
        let hz = self
            .ui
            .refresh_rate_hz
            .clamp(REFRESH_RATE_HZ_MIN, REFRESH_RATE_HZ_MAX);
        Duration::from_micros(1_000_000 / hz as u64)
    }
}

/// Clamp `value` to `min..=max`, noting the change in `changes`.
fn clamp_field<T>(changes: &mut Vec<String>, name: &str, value: &mut T, min: T, max: T)
where
    T: Copy + Ord + std::fmt::Display,
{
    let clamped = (*value).clamp(min, max);
    if clamped != *value {
        changes.push(format!(
            "{} = {} is outside {}..={}, using {}",
            name, value, min, max, clamped
        ));
        *value = clamped;
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
        assert_eq!(config.stickiness.stuck_threshold_ms, 2000);
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

    #[test]
    fn config_load_malformed_toml_returns_parse_error() {
        let path = temp_config_path();
        fs::write(&path, "this is not [valid toml").unwrap();
        let result = Config::load_from(&path);
        assert!(matches!(result, Err(ConfigError::Parse(_))));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn config_save_to_readonly_dir_returns_io_error() {
        let path = PathBuf::from("/proc/nonexistent/config.toml");
        let config = Config::default();
        let result = config.save_to(&path);
        assert!(matches!(result, Err(ConfigError::Io(_))));
    }

    #[test]
    fn config_error_is_std_error() {
        let err: Box<dyn std::error::Error> = Box::new(ConfigError::NoConfigDir);
        assert!(!err.to_string().is_empty());
    }

    fn temp_path(name: &str) -> PathBuf {
        env::temp_dir().join(format!(
            "keyboard-testkit-test-{}-{}.toml",
            name,
            std::process::id()
        ))
    }

    #[test]
    fn default_config_is_already_in_range() {
        let mut config = Config::default();
        assert!(config.sanitize().is_empty());
    }

    #[test]
    fn sanitize_clamps_zero_and_absurd_values() {
        let mut config = Config::default();
        config.ui.refresh_rate_hz = 0;
        config.ui.warning_duration_secs = u32::MAX;
        config.stickiness.stuck_threshold_ms = 0;
        config.polling.test_duration_secs = 0;
        config.polling.sample_window_ms = u64::MAX;
        config.hold_release.bounce_window_ms = 0;

        let changes = config.sanitize();
        assert_eq!(changes.len(), 6, "{:?}", changes);
        assert!(changes[0].contains("polling.test_duration_secs"));
        assert_eq!(config.ui.refresh_rate_hz, REFRESH_RATE_HZ_MIN);
        assert_eq!(config.ui.warning_duration_secs, WARNING_DURATION_SECS_MAX);
        assert_eq!(config.stickiness.stuck_threshold_ms, STUCK_THRESHOLD_MS_MIN);
        assert_eq!(config.polling.test_duration_secs, POLLING_DURATION_SECS_MIN);
        assert_eq!(config.polling.sample_window_ms, SAMPLE_WINDOW_MS_MAX);
        assert_eq!(config.hold_release.bounce_window_ms, BOUNCE_WINDOW_MS_MIN);

        config.ui.refresh_rate_hz = 100_000;
        config.sanitize();
        assert_eq!(config.ui.refresh_rate_hz, REFRESH_RATE_HZ_MAX);
    }

    #[test]
    fn sanitize_keeps_in_range_user_values() {
        let mut config = Config::default();
        config.ui.refresh_rate_hz = 144;
        config.stickiness.stuck_threshold_ms = 5000;
        config.polling.test_duration_secs = 30;
        assert!(config.sanitize().is_empty());
        assert_eq!(config.ui.refresh_rate_hz, 144);
        assert_eq!(config.stickiness.stuck_threshold_ms, 5000);
        assert_eq!(config.polling.test_duration_secs, 30);
    }

    #[test]
    fn refresh_interval_never_divides_by_zero() {
        let mut config = Config::default();
        config.ui.refresh_rate_hz = 0;
        assert_eq!(
            config.refresh_interval(),
            Duration::from_micros(1_000_000 / REFRESH_RATE_HZ_MIN as u64)
        );
    }

    #[test]
    fn load_clamps_hand_edited_values() {
        let path = temp_path("clamp");
        fs::write(
            &path,
            r#"
[polling]
test_duration_secs = 0
sample_window_ms = 100

[stickiness]
stuck_threshold_ms = 0

[hold_release]
bounce_window_ms = 5
min_hold_ms = 10

[ui]
refresh_rate_hz = 0
warning_duration_secs = 3
theme = "Dark"
"#,
        )
        .unwrap();
        let loaded = Config::load_from(&path);
        let _ = fs::remove_file(&path);
        let config = loaded.expect("config with out-of-range values must still load");
        assert_eq!(config.ui.refresh_rate_hz, REFRESH_RATE_HZ_MIN);
        assert_eq!(config.stickiness.stuck_threshold_ms, STUCK_THRESHOLD_MS_MIN);
        assert_eq!(config.polling.test_duration_secs, POLLING_DURATION_SECS_MIN);
        assert!(config.refresh_interval() > Duration::ZERO);
    }

    #[test]
    fn load_partial_config_uses_defaults_for_missing_values() {
        let path = temp_path("partial");
        fs::write(&path, "[ui]\nrefresh_rate_hz = 120\n").unwrap();
        let loaded = Config::load_from(&path);
        let _ = fs::remove_file(&path);
        let config = loaded.expect("partial config must load");
        assert_eq!(config.ui.refresh_rate_hz, 120);
        assert_eq!(config.ui.theme, Theme::Dark);
        assert_eq!(config.stickiness.stuck_threshold_ms, 2000);
        assert!(config.oem_keys.enabled);
    }

    #[test]
    fn load_existing_full_config_file_unchanged() {
        // A file written by an earlier version (every field present)
        let path = temp_path("full");
        let mut original = Config::default();
        original.oem_keys.add_mapping(100, 97);
        original.oem_keys.capture_unknown = false;
        original.save_to(&path).unwrap();
        let loaded = Config::load_from(&path);
        let _ = fs::remove_file(&path);
        let config = loaded.expect("existing config must load");
        assert_eq!(
            toml::to_string_pretty(&config).unwrap(),
            toml::to_string_pretty(&original).unwrap()
        );
    }
}
