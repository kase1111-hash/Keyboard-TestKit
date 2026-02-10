//! OEM key capture and FN key restoration test module
//!
//! This module provides testing and monitoring for:
//! - OEM-specific key presses (media keys, brightness, etc.)
//! - FN key functionality and restoration
//! - Key remapping validation
//! - Unknown/unmapped key detection

use super::{KeyboardTest, ResultStatus, TestResult};
use crate::keyboard::remap::{FnKeyMode, KeyRemapper, RemapResult, RemapStats};
use crate::keyboard::{keymap, KeyCode, KeyEvent, KeyEventType};
use std::collections::{HashMap, VecDeque};
use std::time::Instant;

/// Record of an OEM key event
#[derive(Debug, Clone)]
pub struct OemKeyEvent {
    /// The key code
    pub key: KeyCode,
    /// Human-readable name
    pub name: String,
    /// Event type (press/release)
    pub event_type: KeyEventType,
    /// When it occurred
    pub timestamp: Instant,
    /// Whether it was recognized as an OEM key
    pub is_oem: bool,
    /// Whether it was remapped
    pub was_remapped: bool,
    /// Target key if remapped
    pub remapped_to: Option<KeyCode>,
}

/// Test for OEM key capture and FN key restoration
pub struct OemKeyTest {
    /// Key remapper for handling OEM keys
    remapper: KeyRemapper,
    /// Remapping statistics
    stats: RemapStats,
    /// History of OEM key events
    oem_events: VecDeque<OemKeyEvent>,
    /// Detected OEM keys with press counts
    detected_oem_keys: HashMap<u16, u32>,
    /// Detected unknown keys (not in keymap)
    detected_unknown: HashMap<u16, u32>,
    /// FN key press count
    fn_press_count: u32,
    /// FN combo activations
    fn_combo_count: u32,
    /// Keys remapped count
    remapped_count: u32,
    /// Test start time
    start_time: Option<Instant>,
    /// Last detected OEM key
    last_oem_key: Option<OemKeyEvent>,
    /// Last FN combo result
    last_fn_combo: Option<(KeyCode, KeyCode)>,
}

impl OemKeyTest {
    pub fn new() -> Self {
        Self {
            remapper: KeyRemapper::new(),
            stats: RemapStats::new(),
            oem_events: VecDeque::new(),
            detected_oem_keys: HashMap::new(),
            detected_unknown: HashMap::new(),
            fn_press_count: 0,
            fn_combo_count: 0,
            remapped_count: 0,
            start_time: None,
            last_oem_key: None,
            last_fn_combo: None,
        }
    }

    /// Create with a specific FN mode
    pub fn with_fn_mode(mode: FnKeyMode) -> Self {
        let mut test = Self::new();
        test.remapper.set_fn_mode(mode);
        test
    }

    /// Get reference to the remapper
    pub fn remapper(&self) -> &KeyRemapper {
        &self.remapper
    }

    /// Get mutable reference to the remapper
    pub fn remapper_mut(&mut self) -> &mut KeyRemapper {
        &mut self.remapper
    }

    /// Set the FN key mode
    pub fn set_fn_mode(&mut self, mode: FnKeyMode) {
        self.remapper.set_fn_mode(mode);
    }

    /// Get the current FN mode
    pub fn fn_mode(&self) -> FnKeyMode {
        self.remapper.fn_mode()
    }

    /// Add a custom key mapping
    pub fn add_mapping(&mut self, from: u16, to: u16) {
        self.remapper.add_mapping(from, to);
    }

    /// Add an FN key scancode to recognize
    pub fn add_fn_scancode(&mut self, scancode: u16) {
        self.remapper.add_fn_scancode(scancode);
    }

    /// Add a direct key mapping (remap one key to another)
    pub fn add_direct_mapping(&mut self, from_scancode: u16, to_scancode: u16) {
        self.remapper.add_mapping(from_scancode, to_scancode);
    }

    /// Get the last detected unknown scancode (if any)
    pub fn last_unknown_scancode(&self) -> Option<u16> {
        self.detected_unknown
            .iter()
            .max_by_key(|(_, &count)| count)
            .map(|(&scancode, _)| scancode)
    }

    /// Add an FN+key combination mapping
    pub fn add_fn_combo(&mut self, key_scancode: u16, result_scancode: u16) {
        self.remapper.add_fn_combo(key_scancode, result_scancode);
    }

    /// Check if FN is currently held
    pub fn is_fn_held(&self) -> bool {
        self.remapper.is_fn_held()
    }

    /// Get recently captured OEM/unknown keys
    pub fn recent_captured(&self, count: usize) -> Vec<&OemKeyEvent> {
        self.oem_events.iter().rev().take(count).collect()
    }

    /// Get all detected OEM keys with counts
    pub fn detected_oem_keys(&self) -> &HashMap<u16, u32> {
        &self.detected_oem_keys
    }

    /// Get all detected unknown keys with counts
    pub fn detected_unknown(&self) -> &HashMap<u16, u32> {
        &self.detected_unknown
    }

    /// Apply the current remap result and record the event
    fn record_event(&mut self, key: KeyCode, event_type: KeyEventType, result: &RemapResult) {
        let scancode = key.as_u16();
        let is_oem = keymap::is_oem_key(key);
        let key_info = keymap::get_key_info(key);

        // Track OEM keys
        if is_oem && event_type == KeyEventType::Press {
            *self.detected_oem_keys.entry(scancode).or_insert(0) += 1;
        }

        // Track unknown keys
        if key_info.name == "Unknown" && event_type == KeyEventType::Press {
            *self.detected_unknown.entry(scancode).or_insert(0) += 1;
        }

        // Determine if remapped and to what
        let (was_remapped, remapped_to) = match result {
            RemapResult::Remapped { to, .. } => {
                if event_type == KeyEventType::Press {
                    self.remapped_count += 1;
                }
                (true, Some(*to))
            }
            RemapResult::FnCombo { result, original } => {
                if event_type == KeyEventType::Press {
                    self.fn_combo_count += 1;
                    self.last_fn_combo = Some((*original, *result));
                }
                (true, Some(*result))
            }
            RemapResult::FnModifier { pressed } => {
                if *pressed {
                    self.fn_press_count += 1;
                }
                (false, None)
            }
            _ => (false, None),
        };

        // Create and store OEM event if relevant
        if is_oem || was_remapped || key_info.name == "Unknown" {
            let oem_event = OemKeyEvent {
                key,
                name: key_info.name.to_string(),
                event_type,
                timestamp: Instant::now(),
                is_oem,
                was_remapped,
                remapped_to,
            };

            if event_type == KeyEventType::Press {
                self.last_oem_key = Some(oem_event.clone());
            }

            self.oem_events.push_back(oem_event);

            // Keep only last 100 events
            if self.oem_events.len() > 100 {
                self.oem_events.pop_front();
            }
        }
    }

    /// Get FN mode as display string
    fn fn_mode_display(&self) -> &'static str {
        match self.remapper.fn_mode() {
            FnKeyMode::Disabled => "Disabled",
            FnKeyMode::CaptureOnly => "Capture Only",
            FnKeyMode::RestoreWithModifier => "Restore as Modifier",
            FnKeyMode::MapToFKeys => "Map to F-Keys",
            FnKeyMode::MapToMedia => "Map to Media",
        }
    }
}

impl Default for OemKeyTest {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyboardTest for OemKeyTest {
    fn name(&self) -> &'static str {
        "OEM Key Capture"
    }

    fn description(&self) -> &'static str {
        "Captures OEM-specific keys and provides FN key restoration"
    }

    fn process_event(&mut self, event: &KeyEvent) {
        if self.start_time.is_none() {
            self.start_time = Some(Instant::now());
        }

        // Process through remapper
        let result = self
            .remapper
            .process_key(event.key, event.event_type == KeyEventType::Press);
        self.stats.record(&result);

        // Record the event
        self.record_event(event.key, event.event_type, &result);
    }

    fn is_complete(&self) -> bool {
        false // Continuous test
    }

    fn get_results(&self) -> Vec<TestResult> {
        let mut results = vec![
            // Explanation header
            TestResult::info("--- What This Measures ---", ""),
            TestResult::info("Captures OEM/vendor keys", "(media, brightness, etc.)"),
            TestResult::info("Restores FN key function", "when OEM software removed"),
            TestResult::info("Maps FN+key combos to", "standard function keys"),
            TestResult::info("", ""),
            // FN key status
            TestResult::info("--- FN Key Status ---", ""),
        ];

        let fn_status = if self.remapper.is_fn_held() {
            ResultStatus::Ok
        } else {
            ResultStatus::Info
        };
        results.push(TestResult::new(
            "FN Key Held",
            if self.remapper.is_fn_held() {
                "Yes"
            } else {
                "No"
            },
            fn_status,
        ));

        results.push(TestResult::info("FN Mode", self.fn_mode_display()));

        results.push(TestResult::info(
            "FN Presses",
            format!("{}", self.fn_press_count),
        ));

        results.push(TestResult::info(
            "FN Combos Triggered",
            format!("{}", self.fn_combo_count),
        ));

        // Last FN combo
        if let Some((from, to)) = &self.last_fn_combo {
            let from_info = keymap::get_key_info(*from);
            let to_info = keymap::get_key_info(*to);
            results.push(TestResult::ok(
                "Last FN Combo",
                format!("FN+{} -> {}", from_info.name, to_info.name),
            ));
        }

        results.push(TestResult::info("", ""));

        // OEM key statistics
        results.push(TestResult::info("--- OEM Key Statistics ---", ""));

        results.push(TestResult::info(
            "OEM Keys Detected",
            format!("{}", self.detected_oem_keys.len()),
        ));

        results.push(TestResult::info(
            "Unknown Keys Found",
            format!("{}", self.detected_unknown.len()),
        ));

        results.push(TestResult::info(
            "Keys Remapped",
            format!("{}", self.remapped_count),
        ));

        // Last OEM key
        if let Some(last) = &self.last_oem_key {
            let status = if last.is_oem {
                ResultStatus::Ok
            } else if last.was_remapped {
                ResultStatus::Warning
            } else {
                ResultStatus::Info
            };

            let mut desc = last.name.clone();
            if let Some(target) = last.remapped_to {
                let target_info = keymap::get_key_info(target);
                desc = format!("{} -> {}", desc, target_info.name);
            }

            results.push(TestResult::new("Last OEM Key", desc, status));
        }

        // Recent OEM events
        let recent = self.recent_captured(5);
        if !recent.is_empty() {
            results.push(TestResult::info("", ""));
            results.push(TestResult::info("--- Recent OEM Events ---", ""));

            for evt in recent {
                let action = match evt.event_type {
                    KeyEventType::Press => "pressed",
                    KeyEventType::Release => "released",
                };

                let status = if evt.is_oem {
                    ResultStatus::Ok
                } else if evt.was_remapped {
                    ResultStatus::Warning
                } else {
                    ResultStatus::Info
                };

                let mut desc = format!("{} ({})", evt.name, action);
                if let Some(target) = evt.remapped_to {
                    let target_info = keymap::get_key_info(target);
                    desc = format!("{} -> {}", desc, target_info.name);
                }

                results.push(TestResult::new(
                    format!("  0x{:03X}", evt.key.as_u16()),
                    desc,
                    status,
                ));
            }
        }

        // Unknown keys detected (show scancodes for debugging)
        if !self.detected_unknown.is_empty() {
            results.push(TestResult::info("", ""));
            results.push(TestResult::warning(
                "--- Unknown Keys ---",
                "(add to FN scancodes?)",
            ));

            let mut unknown: Vec<_> = self.detected_unknown.iter().collect();
            unknown.sort_by(|a, b| b.1.cmp(a.1)); // Sort by count

            for (scancode, count) in unknown.iter().take(5) {
                results.push(TestResult::warning(
                    format!("  Scancode 0x{:03X}", scancode),
                    format!("pressed {} time(s)", count),
                ));
            }
        }

        // Active mappings
        let mappings = self.remapper.mappings();
        if !mappings.is_empty() {
            results.push(TestResult::info("", ""));
            results.push(TestResult::info("--- Active Mappings ---", ""));

            for (from, to) in mappings.iter().take(5) {
                let from_info = keymap::get_key_info(KeyCode::new(*from));
                let to_info = keymap::get_key_info(KeyCode::new(*to));
                results.push(TestResult::info(
                    format!("  {}", from_info.name),
                    format!("-> {}", to_info.name),
                ));
            }

            if mappings.len() > 5 {
                results.push(TestResult::info(
                    "",
                    format!("... and {} more", mappings.len() - 5),
                ));
            }
        }

        // Help text
        results.push(TestResult::info("", ""));
        results.push(TestResult::info("--- Controls ---", ""));
        results.push(TestResult::info("[a] Add last unknown", "as FN scancode"));
        results.push(TestResult::info("[f] Cycle FN mode", ""));
        results.push(TestResult::info("[c] Clear mappings", ""));
        results.push(TestResult::info("", ""));
        results.push(TestResult::info("--- Tips ---", ""));
        results.push(TestResult::info(
            "Press unknown keys to",
            "capture their scancodes",
        ));
        results.push(TestResult::info(
            "Add custom mappings in",
            "~/.config/keyboard-testkit/config.toml",
        ));

        results
    }

    fn reset(&mut self) {
        self.stats = RemapStats::new();
        self.oem_events.clear();
        self.detected_oem_keys.clear();
        self.detected_unknown.clear();
        self.fn_press_count = 0;
        self.fn_combo_count = 0;
        self.remapped_count = 0;
        self.start_time = None;
        self.last_oem_key = None;
        self.last_fn_combo = None;
        self.remapper.reset_state();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_event(key: u16, pressed: bool) -> KeyEvent {
        KeyEvent::new(
            KeyCode::new(key),
            if pressed {
                KeyEventType::Press
            } else {
                KeyEventType::Release
            },
            Instant::now(),
            1000,
        )
    }

    #[test]
    fn test_new_oem_test() {
        let test = OemKeyTest::new();
        assert!(!test.is_fn_held());
        assert_eq!(test.fn_press_count, 0);
    }

    #[test]
    fn test_fn_key_detection() {
        let mut test = OemKeyTest::new();

        // Press FN key (scancode 464)
        let event = make_event(464, true);
        test.process_event(&event);

        assert!(test.is_fn_held());
        assert_eq!(test.fn_press_count, 1);

        // Release FN key
        let event = make_event(464, false);
        test.process_event(&event);

        assert!(!test.is_fn_held());
    }

    #[test]
    fn test_fn_combo() {
        let mut test = OemKeyTest::with_fn_mode(FnKeyMode::MapToFKeys);

        // Press FN
        test.process_event(&make_event(464, true));

        // Press 1 (should map to F1)
        test.process_event(&make_event(2, true));

        assert_eq!(test.fn_combo_count, 1);
        assert!(test.last_fn_combo.is_some());

        let (from, to) = test.last_fn_combo.unwrap();
        assert_eq!(from.as_u16(), 2); // Key 1
        assert_eq!(to.as_u16(), 59); // F1
    }

    #[test]
    fn test_oem_key_capture() {
        let mut test = OemKeyTest::new();

        // Press a media key (mute = 113)
        let event = make_event(113, true);
        test.process_event(&event);

        assert_eq!(test.detected_oem_keys.len(), 1);
        assert_eq!(*test.detected_oem_keys.get(&113).unwrap(), 1);
    }

    #[test]
    fn test_unknown_key_capture() {
        let mut test = OemKeyTest::new();

        // Press an unknown key (high scancode)
        let event = make_event(999, true);
        test.process_event(&event);

        assert_eq!(test.detected_unknown.len(), 1);
        assert_eq!(*test.detected_unknown.get(&999).unwrap(), 1);
    }

    #[test]
    fn test_custom_mapping() {
        let mut test = OemKeyTest::new();
        test.add_mapping(58, 1); // CapsLock -> Escape

        let event = make_event(58, true);
        test.process_event(&event);

        assert_eq!(test.remapped_count, 1);
    }

    #[test]
    fn test_reset() {
        let mut test = OemKeyTest::new();

        test.process_event(&make_event(464, true));
        test.process_event(&make_event(113, true));

        assert!(test.is_fn_held());
        assert_eq!(test.fn_press_count, 1);
        assert!(!test.detected_oem_keys.is_empty());

        test.reset();

        assert!(!test.is_fn_held());
        assert_eq!(test.fn_press_count, 0);
        assert!(test.detected_oem_keys.is_empty());
    }

    #[test]
    fn test_get_results() {
        let test = OemKeyTest::new();
        let results = test.get_results();

        assert!(!results.is_empty());

        // Should contain FN status
        let labels: Vec<_> = results.iter().map(|r| r.label.as_str()).collect();
        assert!(labels.contains(&"FN Key Held"));
        assert!(labels.contains(&"FN Mode"));
    }
}
