//! Key code definitions and keyboard layout mapping

use std::collections::HashMap;
use std::sync::LazyLock;

/// Represents a physical key code
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyCode(pub u16);

impl KeyCode {
    pub fn new(code: u16) -> Self {
        Self(code)
    }

    pub fn as_u16(&self) -> u16 {
        self.0
    }
}

impl From<u16> for KeyCode {
    fn from(code: u16) -> Self {
        Self(code)
    }
}

impl From<device_query::Keycode> for KeyCode {
    fn from(keycode: device_query::Keycode) -> Self {
        use device_query::Keycode as DK;
        // Map device_query keycodes to Linux evdev scancodes
        let code = match keycode {
            DK::Escape => 1,
            DK::Key1 => 2,
            DK::Key2 => 3,
            DK::Key3 => 4,
            DK::Key4 => 5,
            DK::Key5 => 6,
            DK::Key6 => 7,
            DK::Key7 => 8,
            DK::Key8 => 9,
            DK::Key9 => 10,
            DK::Key0 => 11,
            DK::Minus => 12,
            DK::Equal => 13,
            DK::Backspace => 14,
            DK::Tab => 15,
            DK::Q => 16,
            DK::W => 17,
            DK::E => 18,
            DK::R => 19,
            DK::T => 20,
            DK::Y => 21,
            DK::U => 22,
            DK::I => 23,
            DK::O => 24,
            DK::P => 25,
            DK::LeftBracket => 26,
            DK::RightBracket => 27,
            DK::Enter => 28,
            DK::LControl => 29,
            DK::A => 30,
            DK::S => 31,
            DK::D => 32,
            DK::F => 33,
            DK::G => 34,
            DK::H => 35,
            DK::J => 36,
            DK::K => 37,
            DK::L => 38,
            DK::Semicolon => 39,
            DK::Apostrophe => 40,
            DK::Grave => 41,
            DK::LShift => 42,
            DK::BackSlash => 43,
            DK::Z => 44,
            DK::X => 45,
            DK::C => 46,
            DK::V => 47,
            DK::B => 48,
            DK::N => 49,
            DK::M => 50,
            DK::Comma => 51,
            DK::Dot => 52,
            DK::Slash => 53,
            DK::RShift => 54,
            DK::LAlt => 56,
            DK::Space => 57,
            DK::CapsLock => 58,
            DK::F1 => 59,
            DK::F2 => 60,
            DK::F3 => 61,
            DK::F4 => 62,
            DK::F5 => 63,
            DK::F6 => 64,
            DK::F7 => 65,
            DK::F8 => 66,
            DK::F9 => 67,
            DK::F10 => 68,
            DK::F11 => 87,
            DK::F12 => 88,
            DK::RControl => 97,
            DK::RAlt => 100,
            DK::Home => 102,
            DK::Up => 103,
            DK::PageUp => 104,
            DK::Left => 105,
            DK::Right => 106,
            DK::End => 107,
            DK::Down => 108,
            DK::PageDown => 109,
            DK::Insert => 110,
            DK::Delete => 111,
            DK::LMeta => 125,
            DK::RMeta => 126,
            // Numpad keys
            DK::Numpad0 => 82,
            DK::Numpad1 => 79,
            DK::Numpad2 => 80,
            DK::Numpad3 => 81,
            DK::Numpad4 => 75,
            DK::Numpad5 => 76,
            DK::Numpad6 => 77,
            DK::Numpad7 => 71,
            DK::Numpad8 => 72,
            DK::Numpad9 => 73,
            DK::NumpadSubtract => 74,
            DK::NumpadAdd => 78,
            DK::NumpadDivide => 98,
            DK::NumpadMultiply => 55,
            // Fallback for any unmapped keys
            _ => 0,
        };
        Self(code)
    }
}

/// Information about a key
#[derive(Debug, Clone)]
pub struct KeyInfo {
    /// Display name for the key
    pub name: &'static str,
    /// Short label (for keyboard visualization)
    pub label: &'static str,
    /// Row position on standard layout (0 = function row, 1-5 = main rows)
    pub row: u8,
    /// Column position on standard layout
    pub col: u8,
    /// Width in units (1.0 = standard key)
    pub width: f32,
}

impl KeyInfo {
    const fn new(name: &'static str, label: &'static str, row: u8, col: u8, width: f32) -> Self {
        Self { name, label, row, col, width }
    }
}

/// Static keymap for standard US keyboard layout
pub static KEYMAP: LazyLock<HashMap<KeyCode, KeyInfo>> = LazyLock::new(|| {
    let mut map = HashMap::new();

    // Function row
    map.insert(KeyCode(1), KeyInfo::new("Escape", "Esc", 0, 0, 1.0));
    map.insert(KeyCode(59), KeyInfo::new("F1", "F1", 0, 2, 1.0));
    map.insert(KeyCode(60), KeyInfo::new("F2", "F2", 0, 3, 1.0));
    map.insert(KeyCode(61), KeyInfo::new("F3", "F3", 0, 4, 1.0));
    map.insert(KeyCode(62), KeyInfo::new("F4", "F4", 0, 5, 1.0));
    map.insert(KeyCode(63), KeyInfo::new("F5", "F5", 0, 7, 1.0));
    map.insert(KeyCode(64), KeyInfo::new("F6", "F6", 0, 8, 1.0));
    map.insert(KeyCode(65), KeyInfo::new("F7", "F7", 0, 9, 1.0));
    map.insert(KeyCode(66), KeyInfo::new("F8", "F8", 0, 10, 1.0));
    map.insert(KeyCode(67), KeyInfo::new("F9", "F9", 0, 12, 1.0));
    map.insert(KeyCode(68), KeyInfo::new("F10", "F10", 0, 13, 1.0));
    map.insert(KeyCode(87), KeyInfo::new("F11", "F11", 0, 14, 1.0));
    map.insert(KeyCode(88), KeyInfo::new("F12", "F12", 0, 15, 1.0));

    // Number row
    map.insert(KeyCode(41), KeyInfo::new("Grave", "`", 1, 0, 1.0));
    map.insert(KeyCode(2), KeyInfo::new("1", "1", 1, 1, 1.0));
    map.insert(KeyCode(3), KeyInfo::new("2", "2", 1, 2, 1.0));
    map.insert(KeyCode(4), KeyInfo::new("3", "3", 1, 3, 1.0));
    map.insert(KeyCode(5), KeyInfo::new("4", "4", 1, 4, 1.0));
    map.insert(KeyCode(6), KeyInfo::new("5", "5", 1, 5, 1.0));
    map.insert(KeyCode(7), KeyInfo::new("6", "6", 1, 6, 1.0));
    map.insert(KeyCode(8), KeyInfo::new("7", "7", 1, 7, 1.0));
    map.insert(KeyCode(9), KeyInfo::new("8", "8", 1, 8, 1.0));
    map.insert(KeyCode(10), KeyInfo::new("9", "9", 1, 9, 1.0));
    map.insert(KeyCode(11), KeyInfo::new("0", "0", 1, 10, 1.0));
    map.insert(KeyCode(12), KeyInfo::new("Minus", "-", 1, 11, 1.0));
    map.insert(KeyCode(13), KeyInfo::new("Equals", "=", 1, 12, 1.0));
    map.insert(KeyCode(14), KeyInfo::new("Backspace", "Bksp", 1, 13, 2.0));

    // Top letter row
    map.insert(KeyCode(15), KeyInfo::new("Tab", "Tab", 2, 0, 1.5));
    map.insert(KeyCode(16), KeyInfo::new("Q", "Q", 2, 1, 1.0));
    map.insert(KeyCode(17), KeyInfo::new("W", "W", 2, 2, 1.0));
    map.insert(KeyCode(18), KeyInfo::new("E", "E", 2, 3, 1.0));
    map.insert(KeyCode(19), KeyInfo::new("R", "R", 2, 4, 1.0));
    map.insert(KeyCode(20), KeyInfo::new("T", "T", 2, 5, 1.0));
    map.insert(KeyCode(21), KeyInfo::new("Y", "Y", 2, 6, 1.0));
    map.insert(KeyCode(22), KeyInfo::new("U", "U", 2, 7, 1.0));
    map.insert(KeyCode(23), KeyInfo::new("I", "I", 2, 8, 1.0));
    map.insert(KeyCode(24), KeyInfo::new("O", "O", 2, 9, 1.0));
    map.insert(KeyCode(25), KeyInfo::new("P", "P", 2, 10, 1.0));
    map.insert(KeyCode(26), KeyInfo::new("LeftBracket", "[", 2, 11, 1.0));
    map.insert(KeyCode(27), KeyInfo::new("RightBracket", "]", 2, 12, 1.0));
    map.insert(KeyCode(43), KeyInfo::new("Backslash", "\\", 2, 13, 1.5));

    // Home row
    map.insert(KeyCode(58), KeyInfo::new("CapsLock", "Caps", 3, 0, 1.75));
    map.insert(KeyCode(30), KeyInfo::new("A", "A", 3, 1, 1.0));
    map.insert(KeyCode(31), KeyInfo::new("S", "S", 3, 2, 1.0));
    map.insert(KeyCode(32), KeyInfo::new("D", "D", 3, 3, 1.0));
    map.insert(KeyCode(33), KeyInfo::new("F", "F", 3, 4, 1.0));
    map.insert(KeyCode(34), KeyInfo::new("G", "G", 3, 5, 1.0));
    map.insert(KeyCode(35), KeyInfo::new("H", "H", 3, 6, 1.0));
    map.insert(KeyCode(36), KeyInfo::new("J", "J", 3, 7, 1.0));
    map.insert(KeyCode(37), KeyInfo::new("K", "K", 3, 8, 1.0));
    map.insert(KeyCode(38), KeyInfo::new("L", "L", 3, 9, 1.0));
    map.insert(KeyCode(39), KeyInfo::new("Semicolon", ";", 3, 10, 1.0));
    map.insert(KeyCode(40), KeyInfo::new("Apostrophe", "'", 3, 11, 1.0));
    map.insert(KeyCode(28), KeyInfo::new("Enter", "Enter", 3, 12, 2.25));

    // Bottom letter row
    map.insert(KeyCode(42), KeyInfo::new("LeftShift", "Shift", 4, 0, 2.25));
    map.insert(KeyCode(44), KeyInfo::new("Z", "Z", 4, 1, 1.0));
    map.insert(KeyCode(45), KeyInfo::new("X", "X", 4, 2, 1.0));
    map.insert(KeyCode(46), KeyInfo::new("C", "C", 4, 3, 1.0));
    map.insert(KeyCode(47), KeyInfo::new("V", "V", 4, 4, 1.0));
    map.insert(KeyCode(48), KeyInfo::new("B", "B", 4, 5, 1.0));
    map.insert(KeyCode(49), KeyInfo::new("N", "N", 4, 6, 1.0));
    map.insert(KeyCode(50), KeyInfo::new("M", "M", 4, 7, 1.0));
    map.insert(KeyCode(51), KeyInfo::new("Comma", ",", 4, 8, 1.0));
    map.insert(KeyCode(52), KeyInfo::new("Period", ".", 4, 9, 1.0));
    map.insert(KeyCode(53), KeyInfo::new("Slash", "/", 4, 10, 1.0));
    map.insert(KeyCode(54), KeyInfo::new("RightShift", "Shift", 4, 11, 2.75));

    // Bottom row (modifiers + space)
    map.insert(KeyCode(29), KeyInfo::new("LeftCtrl", "Ctrl", 5, 0, 1.25));
    map.insert(KeyCode(125), KeyInfo::new("LeftMeta", "Win", 5, 1, 1.25));
    map.insert(KeyCode(56), KeyInfo::new("LeftAlt", "Alt", 5, 2, 1.25));
    map.insert(KeyCode(57), KeyInfo::new("Space", "Space", 5, 3, 6.25));
    map.insert(KeyCode(100), KeyInfo::new("RightAlt", "Alt", 5, 4, 1.25));
    map.insert(KeyCode(126), KeyInfo::new("RightMeta", "Win", 5, 5, 1.25));
    map.insert(KeyCode(127), KeyInfo::new("Menu", "Menu", 5, 6, 1.25));
    map.insert(KeyCode(97), KeyInfo::new("RightCtrl", "Ctrl", 5, 7, 1.25));

    // Arrow keys
    map.insert(KeyCode(103), KeyInfo::new("Up", "↑", 5, 9, 1.0));
    map.insert(KeyCode(105), KeyInfo::new("Left", "←", 6, 8, 1.0));
    map.insert(KeyCode(108), KeyInfo::new("Down", "↓", 6, 9, 1.0));
    map.insert(KeyCode(106), KeyInfo::new("Right", "→", 6, 10, 1.0));

    // Navigation cluster
    map.insert(KeyCode(110), KeyInfo::new("Insert", "Ins", 1, 15, 1.0));
    map.insert(KeyCode(102), KeyInfo::new("Home", "Home", 1, 16, 1.0));
    map.insert(KeyCode(104), KeyInfo::new("PageUp", "PgUp", 1, 17, 1.0));
    map.insert(KeyCode(111), KeyInfo::new("Delete", "Del", 2, 15, 1.0));
    map.insert(KeyCode(107), KeyInfo::new("End", "End", 2, 16, 1.0));
    map.insert(KeyCode(109), KeyInfo::new("PageDown", "PgDn", 2, 17, 1.0));

    map
});

/// Get key info by code, returns a default if not found
pub fn get_key_info(code: KeyCode) -> KeyInfo {
    KEYMAP.get(&code).cloned().unwrap_or_else(|| {
        KeyInfo::new("Unknown", "?", 0, 0, 1.0)
    })
}
