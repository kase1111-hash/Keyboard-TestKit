//! Shortcut detection test module

use super::{KeyboardTest, TestResult, ResultStatus};
use crate::keyboard::{KeyCode, KeyEvent, KeyEventType, keymap};
use std::collections::HashSet;
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
        if self.ctrl { parts.push("Ctrl"); }
        if self.alt { parts.push("Alt"); }
        if self.shift { parts.push("Shift"); }
        if self.super_key { parts.push("Super"); }
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
    shortcut_history: Vec<ShortcutEvent>,
    /// Count of shortcuts detected
    total_shortcuts: u32,
    /// Count of known conflicts detected
    conflict_count: u32,
    /// Test start time
    start_time: Option<Instant>,
    /// Last shortcut for display
    last_shortcut: Option<ShortcutEvent>,
}

impl ShortcutTest {
    pub fn new() -> Self {
        Self {
            modifiers: ModifierState::default(),
            pressed_keys: HashSet::new(),
            shortcut_history: Vec::new(),
            total_shortcuts: 0,
            conflict_count: 0,
            start_time: None,
            last_shortcut: None,
        }
    }

    /// Check if a key is a modifier
    fn is_modifier(key: KeyCode) -> bool {
        matches!(key.0,
            29 | 97 |   // Ctrl (left/right)
            56 | 100 |  // Alt (left/right)
            42 | 54 |   // Shift (left/right)
            125 | 126   // Super/Win (left/right)
        )
    }

    /// Update modifier state based on key event
    fn update_modifiers(&mut self, key: KeyCode, pressed: bool) {
        match key.0 {
            29 | 97 => self.modifiers.ctrl = pressed,
            56 | 100 => self.modifiers.alt = pressed,
            42 | 54 => self.modifiers.shift = pressed,
            125 | 126 => self.modifiers.super_key = pressed,
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
        self.shortcut_history.push(event);

        // Keep only last 50 shortcuts
        if self.shortcut_history.len() > 50 {
            self.shortcut_history.remove(0);
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

        let is_modifier = Self::is_modifier(event.key);

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
        let mut results = Vec::new();

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
            results.push(TestResult::new(
                "Last Shortcut",
                &last.combo,
                status,
            ));
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
    }
}
