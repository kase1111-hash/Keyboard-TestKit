//! Shortcut detection and system shortcut enumeration
//!
//! Detects modifier key combos in real-time and enumerates known
//! system-registered hotkeys (desktop environment shortcuts on Linux).

use super::{KeyboardTest, ResultStatus, TestResult};
use crate::keyboard::{keymap, KeyCode, KeyEvent, KeyEventType};
use std::collections::{HashSet, VecDeque};
use std::time::Instant;

/// Common shortcuts that might cause conflicts
const COMMON_CONFLICTS: &[(&str, &[&str])] = &[
    ("Ctrl+W", &["Close tab/window"]),
    ("Ctrl+Q", &["Quit application"]),
    ("Ctrl+S", &["Save"]),
    ("Ctrl+C", &["Copy / Interrupt"]),
    ("Ctrl+V", &["Paste"]),
    ("Ctrl+Z", &["Undo"]),
    ("Ctrl+Tab", &["Switch tab"]),
    ("Alt+Tab", &["Switch window"]),
    ("Alt+F4", &["Close window"]),
    ("Super+L", &["Lock screen"]),
    ("Ctrl+Alt+Del", &["System menu"]),
    ("Ctrl+Shift+Esc", &["Task manager"]),
    ("PrtSc", &["Screenshot"]),
];

/// A system-registered shortcut discovered by enumeration
#[derive(Debug, Clone)]
pub struct SystemShortcut {
    /// The key combination (e.g., "Super+L")
    pub combo: String,
    /// The application or DE component that owns it
    pub application: String,
    /// What the shortcut does
    pub action: String,
}

/// Enumerate system-registered shortcuts on the current platform.
///
/// On Linux, queries gsettings for GNOME/GTK shortcuts and parses
/// common window manager configs. Returns an empty list on unsupported
/// platforms or if no desktop shortcuts are discoverable.
pub fn enumerate_system_shortcuts() -> Vec<SystemShortcut> {
    #[cfg(target_os = "linux")]
    {
        enumerate_linux_shortcuts()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Vec::new()
    }
}

#[cfg(target_os = "linux")]
fn enumerate_linux_shortcuts() -> Vec<SystemShortcut> {
    let mut shortcuts = Vec::new();

    // Try gsettings for GNOME/GTK desktop shortcuts
    if let Ok(output) = std::process::Command::new("gsettings")
        .args(["list-recursively", "org.gnome.desktop.wm.keybindings"])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if let Some(shortcut) = parse_gsettings_line(line, "Window Manager") {
                    shortcuts.push(shortcut);
                }
            }
        }
    }

    // Try GNOME media keys
    if let Ok(output) = std::process::Command::new("gsettings")
        .args([
            "list-recursively",
            "org.gnome.settings-daemon.plugins.media-keys",
        ])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if let Some(shortcut) = parse_gsettings_line(line, "Media Keys") {
                    shortcuts.push(shortcut);
                }
            }
        }
    }

    // Try GNOME shell keybindings
    if let Ok(output) = std::process::Command::new("gsettings")
        .args(["list-recursively", "org.gnome.shell.keybindings"])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if let Some(shortcut) = parse_gsettings_line(line, "GNOME Shell") {
                    shortcuts.push(shortcut);
                }
            }
        }
    }

    shortcuts
}

#[cfg(target_os = "linux")]
fn parse_gsettings_line(line: &str, application: &str) -> Option<SystemShortcut> {
    // Format: "org.gnome.desktop.wm.keybindings close ['<Alt>F4']"
    let parts: Vec<&str> = line.splitn(3, ' ').collect();
    if parts.len() < 3 {
        return None;
    }

    let action = parts[1].to_string();
    let value = parts[2].trim();

    // Skip disabled shortcuts (empty arrays or ['disabled'])
    if value == "@as []" || value.contains("disabled") || value == "['']" {
        return None;
    }

    // Extract key combo from gsettings array format ['<Super>l', '<Super>L']
    let combo = value
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .next()?
        .trim()
        .trim_matches('\'')
        .to_string();

    if combo.is_empty() {
        return None;
    }

    // Convert gsettings format (<Super>l) to display format (Super+L)
    let display_combo = combo
        .replace("<Super>", "Super+")
        .replace("<Primary>", "Ctrl+")
        .replace("<Control>", "Ctrl+")
        .replace("<Alt>", "Alt+")
        .replace("<Shift>", "Shift+")
        .replace("<Meta>", "Meta+");

    // Make the action name human-readable
    let display_action = action.replace(['-', '_'], " ");

    Some(SystemShortcut {
        combo: display_combo,
        application: application.to_string(),
        action: display_action,
    })
}

/// A detected shortcut combination
#[derive(Debug, Clone)]
pub struct ShortcutEvent {
    /// The key combination as a string
    pub combo: String,
    /// When it was detected
    pub timestamp: Instant,
    /// Whether it's a known conflict
    pub is_known_conflict: bool,
    /// Description if known
    pub description: Option<String>,
}

/// Modifier key state
#[derive(Debug, Clone, Default)]
struct ModifierState {
    ctrl: bool,
    alt: bool,
    shift: bool,
    super_key: bool,
}

impl ModifierState {
    fn any_held(&self) -> bool {
        self.ctrl || self.alt || self.shift || self.super_key
    }

    fn to_prefix(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.super_key {
            parts.push("Super");
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!("{}+", parts.join("+"))
        }
    }
}

/// Test for detecting keyboard shortcuts
pub struct ShortcutTest {
    /// Current modifier state
    modifiers: ModifierState,
    /// Currently pressed non-modifier keys
    pressed_keys: HashSet<KeyCode>,
    /// History of detected shortcuts
    shortcut_history: VecDeque<ShortcutEvent>,
    /// Count of shortcuts detected
    total_shortcuts: u32,
    /// Count of known conflicts detected
    conflict_count: u32,
    /// Test start time
    start_time: Option<Instant>,
    /// Last shortcut for display
    last_shortcut: Option<ShortcutEvent>,
    /// System-registered shortcuts discovered at startup
    system_shortcuts: Vec<SystemShortcut>,
}

impl ShortcutTest {
    pub fn new() -> Self {
        let system_shortcuts = enumerate_system_shortcuts();
        Self {
            modifiers: ModifierState::default(),
            pressed_keys: HashSet::new(),
            shortcut_history: VecDeque::new(),
            total_shortcuts: 0,
            conflict_count: 0,
            start_time: None,
            last_shortcut: None,
            system_shortcuts,
        }
    }

    /// Get system-registered shortcuts
    pub fn system_shortcuts(&self) -> &[SystemShortcut] {
        &self.system_shortcuts
    }

    /// Update modifier state based on key event
    fn update_modifiers(&mut self, key: KeyCode, pressed: bool) {
        use crate::keyboard::keymap::*;
        match key {
            KEY_LCTRL | KEY_RCTRL => self.modifiers.ctrl = pressed,
            KEY_LALT | KEY_RALT => self.modifiers.alt = pressed,
            KEY_LSHIFT | KEY_RSHIFT => self.modifiers.shift = pressed,
            KEY_LSUPER | KEY_RSUPER => self.modifiers.super_key = pressed,
            _ => {}
        }
    }

    /// Get key name for display
    fn get_key_name(key: KeyCode) -> String {
        let info = keymap::get_key_info(key);
        info.name.to_string()
    }

    /// Check if a combo is a known conflict
    fn check_known_conflict(combo: &str) -> Option<&'static str> {
        for (pattern, descriptions) in COMMON_CONFLICTS {
            if combo.eq_ignore_ascii_case(pattern) {
                return Some(descriptions[0]);
            }
        }
        None
    }

    /// Record a shortcut event
    fn record_shortcut(&mut self, key: KeyCode, timestamp: Instant) {
        let prefix = self.modifiers.to_prefix();
        if prefix.is_empty() {
            return; // No modifiers held, not a shortcut
        }

        let key_name = Self::get_key_name(key);
        let combo = format!("{}{}", prefix, key_name);

        let description = Self::check_known_conflict(&combo);
        let is_known = description.is_some();

        let event = ShortcutEvent {
            combo: combo.clone(),
            timestamp,
            is_known_conflict: is_known,
            description: description.map(|s| s.to_string()),
        };

        self.total_shortcuts += 1;
        if is_known {
            self.conflict_count += 1;
        }

        self.last_shortcut = Some(event.clone());
        self.shortcut_history.push_back(event);

        // Keep only last 50 shortcuts
        if self.shortcut_history.len() > 50 {
            self.shortcut_history.pop_front();
        }
    }

    /// Get recent shortcuts (last N)
    pub fn recent_shortcuts(&self, count: usize) -> Vec<&ShortcutEvent> {
        self.shortcut_history.iter().rev().take(count).collect()
    }
}

impl Default for ShortcutTest {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyboardTest for ShortcutTest {
    fn name(&self) -> &'static str {
        "Shortcut Detection"
    }

    fn description(&self) -> &'static str {
        "Detects keyboard shortcuts and potential conflicts"
    }

    fn process_event(&mut self, event: &KeyEvent) {
        if self.start_time.is_none() {
            self.start_time = Some(Instant::now());
        }

        let is_modifier = keymap::is_modifier(event.key);

        match event.event_type {
            KeyEventType::Press => {
                if is_modifier {
                    self.update_modifiers(event.key, true);
                } else {
                    // Non-modifier key pressed while modifiers held = shortcut
                    if self.modifiers.any_held() {
                        self.record_shortcut(event.key, event.timestamp);
                    }
                    self.pressed_keys.insert(event.key);
                }
            }
            KeyEventType::Release => {
                if is_modifier {
                    self.update_modifiers(event.key, false);
                } else {
                    self.pressed_keys.remove(&event.key);
                }
            }
        }
    }

    fn is_complete(&self) -> bool {
        false // Continuous test
    }

    fn get_results(&self) -> Vec<TestResult> {
        let mut results = vec![
            // Tooltip: Explain what this test measures
            TestResult::info("--- What This Measures ---", ""),
            TestResult::info("Detects modifier combos", "(Ctrl/Alt/Shift + keys)"),
            TestResult::info("Flags shortcuts that may", "conflict with apps/system"),
            TestResult::info("Look for: working combos,", "no unexpected conflicts"),
            TestResult::info("", ""),
        ];

        // Current modifier state
        let mod_status = if self.modifiers.any_held() {
            self.modifiers.to_prefix().trim_end_matches('+').to_string()
        } else {
            "None".to_string()
        };
        results.push(TestResult::info("Modifiers Held", mod_status));

        // Stats
        results.push(TestResult::info(
            "Total Shortcuts",
            format!("{}", self.total_shortcuts),
        ));

        if self.conflict_count > 0 {
            results.push(TestResult::warning(
                "Known Conflicts",
                format!("{}", self.conflict_count),
            ));
        } else {
            results.push(TestResult::ok("Known Conflicts", "0"));
        }

        // Last shortcut
        if let Some(last) = &self.last_shortcut {
            let status = if last.is_known_conflict {
                ResultStatus::Warning
            } else {
                ResultStatus::Info
            };
            results.push(TestResult::new("Last Shortcut", &last.combo, status));
            if let Some(desc) = &last.description {
                results.push(TestResult::warning("  Action", desc.clone()));
            }
        }

        // Recent shortcuts
        let recent = self.recent_shortcuts(8);
        if !recent.is_empty() {
            results.push(TestResult::info("--- Recent Shortcuts ---", ""));
            for shortcut in recent {
                let status = if shortcut.is_known_conflict {
                    ResultStatus::Warning
                } else {
                    ResultStatus::Info
                };
                let value = if let Some(desc) = &shortcut.description {
                    format!("{} ({})", shortcut.combo, desc)
                } else {
                    shortcut.combo.clone()
                };
                results.push(TestResult::new("  ", value, status));
            }
        }

        // System-registered shortcuts (if discovered)
        if !self.system_shortcuts.is_empty() {
            results.push(TestResult::info("--- System Shortcuts ---", ""));
            results.push(TestResult::info(
                "Discovered",
                format!("{} registered", self.system_shortcuts.len()),
            ));
            for shortcut in self.system_shortcuts.iter().take(10) {
                results.push(TestResult::info(
                    format!("  {}", shortcut.combo),
                    format!("{} ({})", shortcut.action, shortcut.application),
                ));
            }
            if self.system_shortcuts.len() > 10 {
                results.push(TestResult::info(
                    "  ...",
                    format!("{} more", self.system_shortcuts.len() - 10),
                ));
            }
        } else {
            results.push(TestResult::info("--- System Shortcuts ---", ""));
            results.push(TestResult::info(
                "  Not available",
                "gsettings not found",
            ));
        }

        // Known conflicts reference
        results.push(TestResult::info("--- Common Conflicts ---", ""));
        for (combo, _) in COMMON_CONFLICTS.iter().take(5) {
            results.push(TestResult::info("  ", combo.to_string()));
        }

        results
    }

    fn reset(&mut self) {
        self.modifiers = ModifierState::default();
        self.pressed_keys.clear();
        self.shortcut_history.clear();
        self.total_shortcuts = 0;
        self.conflict_count = 0;
        self.start_time = None;
        self.last_shortcut = None;
        // system_shortcuts preserved — they don't change during a session
    }
}
