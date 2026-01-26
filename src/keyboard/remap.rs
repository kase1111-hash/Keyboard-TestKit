//! Key remapping and OEM/FN key restoration module
//!
//! This module provides functionality for:
//! - Capturing and identifying OEM-specific key presses
//! - Remapping keys to different scancodes
//! - Restoring FN key functionality when OEM software is removed
//!
//! ## Usage
//!
//! ```no_run
//! use keyboard_testkit::keyboard::remap::{KeyRemapper, FnKeyMode};
//!
//! // Create a remapper with default settings
//! let mut remapper = KeyRemapper::new();
//!
//! // Add custom key mapping (e.g., map CapsLock to Escape)
//! remapper.add_mapping(58, 1); // CapsLock (58) -> Escape (1)
//!
//! // Enable FN key restoration
//! remapper.set_fn_mode(FnKeyMode::RestoreWithModifier);
//! ```

use super::KeyCode;
use std::collections::HashMap;
use std::time::Instant;

/// Describes how the FN key should be handled
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FnKeyMode {
    /// Don't modify FN key behavior
    Disabled,
    /// Capture FN key presses but don't remap
    #[default]
    CaptureOnly,
    /// Restore FN key as a modifier (like Ctrl/Alt)
    RestoreWithModifier,
    /// Map FN+key combinations to F-keys
    MapToFKeys,
    /// Map FN+key combinations to media keys
    MapToMedia,
}

/// Describes how to handle unmapped/unknown key presses
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UnknownKeyBehavior {
    /// Pass through unchanged
    PassThrough,
    /// Capture and log but pass through
    #[default]
    CaptureAndPassThrough,
    /// Block unknown keys
    Block,
}

/// Record of a captured OEM/unknown key press
#[derive(Debug, Clone)]
pub struct CapturedKey {
    /// Original scancode
    pub scancode: u16,
    /// When it was captured
    pub timestamp: Instant,
    /// Whether it was pressed (true) or released (false)
    pub pressed: bool,
    /// Number of times this key has been pressed
    pub press_count: u32,
    /// Human-readable label if known
    pub label: Option<String>,
}

impl CapturedKey {
    pub fn new(scancode: u16, pressed: bool, label: Option<String>) -> Self {
        Self {
            scancode,
            timestamp: Instant::now(),
            pressed,
            press_count: if pressed { 1 } else { 0 },
            label,
        }
    }
}

/// Result of applying a remap operation
#[derive(Debug, Clone)]
pub enum RemapResult {
    /// Key was not modified
    Unchanged(KeyCode),
    /// Key was remapped to a new code
    Remapped { from: KeyCode, to: KeyCode },
    /// Key was blocked and should not be processed
    Blocked(KeyCode),
    /// This is an FN key modifier state change
    FnModifier { pressed: bool },
    /// FN+key combination was translated
    FnCombo { original: KeyCode, result: KeyCode },
}

/// Handles key remapping and FN key restoration
#[derive(Debug, Clone)]
pub struct KeyRemapper {
    /// Direct key mappings: source scancode -> target scancode
    mappings: HashMap<u16, u16>,
    /// FN key handling mode
    fn_mode: FnKeyMode,
    /// Whether the FN key is currently held
    fn_held: bool,
    /// Scancodes to treat as FN key
    fn_scancodes: Vec<u16>,
    /// Unknown key handling behavior
    unknown_behavior: UnknownKeyBehavior,
    /// History of captured OEM/unknown keys
    captured_keys: HashMap<u16, CapturedKey>,
    /// FN+key to result mappings
    fn_combos: HashMap<u16, u16>,
    /// Whether remapping is enabled
    enabled: bool,
}

impl Default for KeyRemapper {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyRemapper {
    /// Create a new key remapper with default settings
    pub fn new() -> Self {
        let mut remapper = Self {
            mappings: HashMap::new(),
            fn_mode: FnKeyMode::CaptureOnly,
            fn_held: false,
            fn_scancodes: vec![464, 480], // Common FN key scancodes
            unknown_behavior: UnknownKeyBehavior::CaptureAndPassThrough,
            captured_keys: HashMap::new(),
            fn_combos: HashMap::new(),
            enabled: true,
        };

        // Set up default FN+key combos (number row -> F-keys)
        remapper.setup_default_fn_combos();

        remapper
    }

    /// Set up default FN+key combinations
    fn setup_default_fn_combos(&mut self) {
        // FN + number row -> F1-F10
        self.fn_combos.insert(2, 59);   // 1 -> F1
        self.fn_combos.insert(3, 60);   // 2 -> F2
        self.fn_combos.insert(4, 61);   // 3 -> F3
        self.fn_combos.insert(5, 62);   // 4 -> F4
        self.fn_combos.insert(6, 63);   // 5 -> F5
        self.fn_combos.insert(7, 64);   // 6 -> F6
        self.fn_combos.insert(8, 65);   // 7 -> F7
        self.fn_combos.insert(9, 66);   // 8 -> F8
        self.fn_combos.insert(10, 67);  // 9 -> F9
        self.fn_combos.insert(11, 68);  // 0 -> F10
        self.fn_combos.insert(12, 87);  // - -> F11
        self.fn_combos.insert(13, 88);  // = -> F12

        // FN + arrow keys -> media controls (common laptop layout)
        self.fn_combos.insert(105, 165); // Left -> Previous
        self.fn_combos.insert(106, 163); // Right -> Next
        self.fn_combos.insert(103, 115); // Up -> Volume Up
        self.fn_combos.insert(108, 114); // Down -> Volume Down

        // FN + other common mappings
        self.fn_combos.insert(57, 164);  // Space -> Play/Pause
        self.fn_combos.insert(14, 111);  // Backspace -> Delete
        self.fn_combos.insert(1, 142);   // Esc -> Sleep
    }

    /// Enable or disable remapping
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Check if remapping is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Set the FN key handling mode
    pub fn set_fn_mode(&mut self, mode: FnKeyMode) {
        self.fn_mode = mode;
    }

    /// Get the current FN key mode
    pub fn fn_mode(&self) -> FnKeyMode {
        self.fn_mode
    }

    /// Set custom FN key scancodes
    pub fn set_fn_scancodes(&mut self, scancodes: Vec<u16>) {
        self.fn_scancodes = scancodes;
    }

    /// Add an FN key scancode to recognize
    pub fn add_fn_scancode(&mut self, scancode: u16) {
        if !self.fn_scancodes.contains(&scancode) {
            self.fn_scancodes.push(scancode);
        }
    }

    /// Check if a scancode is an FN key
    pub fn is_fn_key(&self, scancode: u16) -> bool {
        self.fn_scancodes.contains(&scancode)
    }

    /// Check if FN key is currently held
    pub fn is_fn_held(&self) -> bool {
        self.fn_held
    }

    /// Set the unknown key handling behavior
    pub fn set_unknown_behavior(&mut self, behavior: UnknownKeyBehavior) {
        self.unknown_behavior = behavior;
    }

    /// Add a key mapping (source -> target)
    pub fn add_mapping(&mut self, from: u16, to: u16) {
        self.mappings.insert(from, to);
    }

    /// Remove a key mapping
    pub fn remove_mapping(&mut self, from: u16) -> Option<u16> {
        self.mappings.remove(&from)
    }

    /// Clear all key mappings
    pub fn clear_mappings(&mut self) {
        self.mappings.clear();
    }

    /// Get all current mappings
    pub fn mappings(&self) -> &HashMap<u16, u16> {
        &self.mappings
    }

    /// Add an FN+key combination mapping
    pub fn add_fn_combo(&mut self, key: u16, result: u16) {
        self.fn_combos.insert(key, result);
    }

    /// Remove an FN+key combination
    pub fn remove_fn_combo(&mut self, key: u16) -> Option<u16> {
        self.fn_combos.remove(&key)
    }

    /// Get all FN+key combinations
    pub fn fn_combos(&self) -> &HashMap<u16, u16> {
        &self.fn_combos
    }

    /// Get captured/unknown keys
    pub fn captured_keys(&self) -> &HashMap<u16, CapturedKey> {
        &self.captured_keys
    }

    /// Clear captured key history
    pub fn clear_captured(&mut self) {
        self.captured_keys.clear();
    }

    /// Process a key event and return the remap result
    pub fn process_key(&mut self, key: KeyCode, pressed: bool) -> RemapResult {
        let scancode = key.as_u16();

        // Check if this is an FN key
        if self.is_fn_key(scancode) {
            self.fn_held = pressed;

            // Capture the FN key press
            self.capture_key(scancode, pressed, Some("Fn".to_string()));

            return RemapResult::FnModifier { pressed };
        }

        // Handle FN+key combinations when FN is held
        if self.fn_held && pressed && self.fn_mode != FnKeyMode::Disabled {
            if let Some(&result) = self.fn_combos.get(&scancode) {
                // Capture the original key
                self.capture_key(scancode, pressed, None);

                return RemapResult::FnCombo {
                    original: key,
                    result: KeyCode::new(result),
                };
            }
        }

        // Check for direct key mappings
        if self.enabled {
            if let Some(&target) = self.mappings.get(&scancode) {
                return RemapResult::Remapped {
                    from: key,
                    to: KeyCode::new(target),
                };
            }
        }

        // Check if this is an unknown/OEM key
        if super::keymap::KEYMAP.get(&key).is_none() {
            self.capture_key(scancode, pressed, None);

            if self.unknown_behavior == UnknownKeyBehavior::Block {
                return RemapResult::Blocked(key);
            }
        }

        RemapResult::Unchanged(key)
    }

    /// Capture an OEM/unknown key for tracking
    fn capture_key(&mut self, scancode: u16, pressed: bool, label: Option<String>) {
        if let Some(existing) = self.captured_keys.get_mut(&scancode) {
            if pressed {
                existing.press_count += 1;
            }
            existing.timestamp = Instant::now();
            existing.pressed = pressed;
            if label.is_some() {
                existing.label = label;
            }
        } else {
            self.captured_keys.insert(
                scancode,
                CapturedKey::new(scancode, pressed, label),
            );
        }
    }

    /// Get recently captured keys sorted by timestamp (most recent first)
    pub fn recent_captured(&self, count: usize) -> Vec<&CapturedKey> {
        let mut keys: Vec<_> = self.captured_keys.values().collect();
        keys.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        keys.into_iter().take(count).collect()
    }

    /// Reset the remapper state (but keep mappings)
    pub fn reset_state(&mut self) {
        self.fn_held = false;
        self.captured_keys.clear();
    }

    /// Create a remapper with common OEM key fixes
    pub fn with_common_oem_fixes() -> Self {
        let mut remapper = Self::new();

        // Common fixes for keyboards that lose functionality after OEM software removal
        remapper.set_fn_mode(FnKeyMode::MapToFKeys);

        remapper
    }

    /// Load mappings from a vector of (from, to) pairs
    pub fn load_mappings(&mut self, mappings: &[(u16, u16)]) {
        for &(from, to) in mappings {
            self.mappings.insert(from, to);
        }
    }

    /// Export current mappings as a vector of (from, to) pairs
    pub fn export_mappings(&self) -> Vec<(u16, u16)> {
        self.mappings.iter().map(|(&k, &v)| (k, v)).collect()
    }
}

/// Statistics about remapping operations
#[derive(Debug, Clone, Default)]
pub struct RemapStats {
    /// Total keys processed
    pub total_processed: u64,
    /// Keys that were remapped
    pub remapped_count: u64,
    /// FN combos triggered
    pub fn_combo_count: u64,
    /// Unknown keys captured
    pub unknown_captured: u64,
    /// Keys blocked
    pub blocked_count: u64,
}

impl RemapStats {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a remap result
    pub fn record(&mut self, result: &RemapResult) {
        self.total_processed += 1;
        match result {
            RemapResult::Remapped { .. } => self.remapped_count += 1,
            RemapResult::FnCombo { .. } => self.fn_combo_count += 1,
            RemapResult::Blocked(_) => self.blocked_count += 1,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_remapper() {
        let remapper = KeyRemapper::new();
        assert!(remapper.is_enabled());
        assert_eq!(remapper.fn_mode(), FnKeyMode::CaptureOnly);
        assert!(!remapper.is_fn_held());
    }

    #[test]
    fn test_add_mapping() {
        let mut remapper = KeyRemapper::new();
        remapper.add_mapping(58, 1); // CapsLock -> Escape

        let result = remapper.process_key(KeyCode::new(58), true);
        match result {
            RemapResult::Remapped { from, to } => {
                assert_eq!(from.as_u16(), 58);
                assert_eq!(to.as_u16(), 1);
            }
            _ => panic!("Expected Remapped result"),
        }
    }

    #[test]
    fn test_remove_mapping() {
        let mut remapper = KeyRemapper::new();
        remapper.add_mapping(58, 1);

        let removed = remapper.remove_mapping(58);
        assert_eq!(removed, Some(1));

        let result = remapper.process_key(KeyCode::new(58), true);
        match result {
            RemapResult::Unchanged(_) => {}
            _ => panic!("Expected Unchanged result after removing mapping"),
        }
    }

    #[test]
    fn test_fn_key_detection() {
        let mut remapper = KeyRemapper::new();

        // Press FN key
        let result = remapper.process_key(KeyCode::new(464), true);
        match result {
            RemapResult::FnModifier { pressed } => assert!(pressed),
            _ => panic!("Expected FnModifier result"),
        }
        assert!(remapper.is_fn_held());

        // Release FN key
        let result = remapper.process_key(KeyCode::new(464), false);
        match result {
            RemapResult::FnModifier { pressed } => assert!(!pressed),
            _ => panic!("Expected FnModifier result"),
        }
        assert!(!remapper.is_fn_held());
    }

    #[test]
    fn test_fn_combo() {
        let mut remapper = KeyRemapper::new();
        remapper.set_fn_mode(FnKeyMode::MapToFKeys);

        // Press FN
        remapper.process_key(KeyCode::new(464), true);

        // Press 1 while FN held -> should get F1
        let result = remapper.process_key(KeyCode::new(2), true);
        match result {
            RemapResult::FnCombo { original, result } => {
                assert_eq!(original.as_u16(), 2);
                assert_eq!(result.as_u16(), 59); // F1
            }
            _ => panic!("Expected FnCombo result"),
        }
    }

    #[test]
    fn test_disabled_remapping() {
        let mut remapper = KeyRemapper::new();
        remapper.add_mapping(58, 1);
        remapper.set_enabled(false);

        let result = remapper.process_key(KeyCode::new(58), true);
        match result {
            RemapResult::Unchanged(_) => {}
            _ => panic!("Expected Unchanged when disabled"),
        }
    }

    #[test]
    fn test_captured_keys() {
        let mut remapper = KeyRemapper::new();

        // Process an unknown key (high scancode)
        remapper.process_key(KeyCode::new(999), true);

        assert_eq!(remapper.captured_keys().len(), 1);
        let captured = remapper.captured_keys().get(&999).unwrap();
        assert_eq!(captured.scancode, 999);
        assert_eq!(captured.press_count, 1);
    }

    #[test]
    fn test_remap_stats() {
        let mut stats = RemapStats::new();

        stats.record(&RemapResult::Remapped {
            from: KeyCode::new(58),
            to: KeyCode::new(1),
        });
        stats.record(&RemapResult::FnCombo {
            original: KeyCode::new(2),
            result: KeyCode::new(59),
        });
        stats.record(&RemapResult::Unchanged(KeyCode::new(30)));

        assert_eq!(stats.total_processed, 3);
        assert_eq!(stats.remapped_count, 1);
        assert_eq!(stats.fn_combo_count, 1);
    }

    #[test]
    fn test_export_import_mappings() {
        let mut remapper = KeyRemapper::new();
        remapper.add_mapping(58, 1);
        remapper.add_mapping(42, 29);

        let exported = remapper.export_mappings();
        assert_eq!(exported.len(), 2);

        let mut new_remapper = KeyRemapper::new();
        new_remapper.load_mappings(&exported);

        assert_eq!(new_remapper.mappings().len(), 2);
        assert_eq!(new_remapper.mappings().get(&58), Some(&1));
        assert_eq!(new_remapper.mappings().get(&42), Some(&29));
    }
}
