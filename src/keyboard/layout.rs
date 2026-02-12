//! Keyboard layout detection and definitions
//!
//! Detects the active keyboard physical layout (ANSI, ISO, JIS) at startup
//! and provides layout-specific key definitions for the visual rendering.

use serde::{Deserialize, Serialize};

/// Physical keyboard layout variant
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyboardLayout {
    /// US ANSI layout (default) - standard US keyboard
    #[default]
    Ansi,
    /// ISO layout (UK, DE, FR, etc.) - L-shaped Enter, extra key left of Z
    Iso,
    /// JIS layout (Japanese) - shorter space, extra keys on bottom row
    Jis,
}

impl KeyboardLayout {
    /// Detect the keyboard layout from the system.
    ///
    /// On Linux: queries `setxkbmap` or reads `/etc/default/keyboard`.
    /// Falls back to ANSI if detection fails.
    pub fn detect() -> Self {
        #[cfg(target_os = "linux")]
        {
            detect_linux_layout()
        }
        #[cfg(not(target_os = "linux"))]
        {
            Self::Ansi
        }
    }

    /// Display name for the layout
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ansi => "ANSI (US)",
            Self::Iso => "ISO (EU)",
            Self::Jis => "JIS (JP)",
        }
    }

    /// Whether the layout has an extra key between left shift and Z (scancode 86)
    pub fn has_iso_key(&self) -> bool {
        matches!(self, Self::Iso)
    }

    /// Whether the layout has extra keys on the bottom row (Henkan, Muhenkan, etc.)
    pub fn has_jis_keys(&self) -> bool {
        matches!(self, Self::Jis)
    }
}

/// Key definition for rendering in the keyboard visual
pub struct VisualKey {
    /// Label to display
    pub label: &'static str,
    /// Evdev scancode
    pub code: u16,
    /// Width in character cells
    pub width: u16,
}

/// Get the key rows for a given layout.
///
/// Returns 5 rows (number row, QWERTY, home, shift, bottom).
/// Each row is a Vec of VisualKey definitions.
pub fn layout_rows(layout: KeyboardLayout) -> [Vec<VisualKey>; 5] {
    let w = 4;

    // Row 0: Number row (same for all layouts)
    let row0 = vec![
        VisualKey { label: "`", code: 41, width: w },
        VisualKey { label: "1", code: 2, width: w },
        VisualKey { label: "2", code: 3, width: w },
        VisualKey { label: "3", code: 4, width: w },
        VisualKey { label: "4", code: 5, width: w },
        VisualKey { label: "5", code: 6, width: w },
        VisualKey { label: "6", code: 7, width: w },
        VisualKey { label: "7", code: 8, width: w },
        VisualKey { label: "8", code: 9, width: w },
        VisualKey { label: "9", code: 10, width: w },
        VisualKey { label: "0", code: 11, width: w },
        VisualKey { label: "-", code: 12, width: w },
        VisualKey { label: "=", code: 13, width: w },
        VisualKey { label: "\u{2190}", code: 14, width: w + 2 }, // ←
    ];

    // Row 1: QWERTY row
    let mut row1 = vec![
        VisualKey { label: "\u{21E5}", code: 15, width: w }, // ⇥
        VisualKey { label: "Q", code: 16, width: w },
        VisualKey { label: "W", code: 17, width: w },
        VisualKey { label: "E", code: 18, width: w },
        VisualKey { label: "R", code: 19, width: w },
        VisualKey { label: "T", code: 20, width: w },
        VisualKey { label: "Y", code: 21, width: w },
        VisualKey { label: "U", code: 22, width: w },
        VisualKey { label: "I", code: 23, width: w },
        VisualKey { label: "O", code: 24, width: w },
        VisualKey { label: "P", code: 25, width: w },
        VisualKey { label: "[", code: 26, width: w },
        VisualKey { label: "]", code: 27, width: w },
    ];
    if layout != KeyboardLayout::Iso {
        // ANSI and JIS have backslash on row 1
        row1.push(VisualKey { label: "\\", code: 43, width: w });
    }

    // Row 2: Home row
    let mut row2 = vec![
        VisualKey { label: "\u{21EA}", code: 58, width: w + 1 }, // ⇪
        VisualKey { label: "A", code: 30, width: w },
        VisualKey { label: "S", code: 31, width: w },
        VisualKey { label: "D", code: 32, width: w },
        VisualKey { label: "F", code: 33, width: w },
        VisualKey { label: "G", code: 34, width: w },
        VisualKey { label: "H", code: 35, width: w },
        VisualKey { label: "J", code: 36, width: w },
        VisualKey { label: "K", code: 37, width: w },
        VisualKey { label: "L", code: 38, width: w },
        VisualKey { label: ";", code: 39, width: w },
        VisualKey { label: "'", code: 40, width: w },
    ];
    if layout == KeyboardLayout::Iso {
        // ISO: backslash/hash key next to Enter (scancode 43)
        row2.push(VisualKey { label: "#", code: 43, width: w });
    }
    // Enter key (wider on ANSI, but we show same width in TUI)
    row2.push(VisualKey { label: "\u{21B5}", code: 28, width: w + 2 }); // ↵

    // Row 3: Shift row
    let mut row3 = vec![];
    if layout == KeyboardLayout::Iso {
        // ISO: shorter left shift + extra key (scancode 86)
        row3.push(VisualKey { label: "\u{21E7}", code: 42, width: w }); // ⇧
        row3.push(VisualKey { label: "\\", code: 86, width: w }); // ISO extra key
    } else {
        // ANSI/JIS: normal left shift
        row3.push(VisualKey { label: "\u{21E7}", code: 42, width: w + 2 }); // ⇧
    }
    row3.extend([
        VisualKey { label: "Z", code: 44, width: w },
        VisualKey { label: "X", code: 45, width: w },
        VisualKey { label: "C", code: 46, width: w },
        VisualKey { label: "V", code: 47, width: w },
        VisualKey { label: "B", code: 48, width: w },
        VisualKey { label: "N", code: 49, width: w },
        VisualKey { label: "M", code: 50, width: w },
        VisualKey { label: ",", code: 51, width: w },
        VisualKey { label: ".", code: 52, width: w },
        VisualKey { label: "/", code: 53, width: w },
    ]);
    row3.push(VisualKey { label: "\u{21E7}", code: 54, width: w + 3 }); // ⇧ right shift

    // Row 4: Bottom row
    let mut row4 = vec![
        VisualKey { label: "Ctl", code: 29, width: w },
        VisualKey { label: "\u{25C6}", code: 125, width: w }, // ◆
        VisualKey { label: "Alt", code: 56, width: w },
    ];

    if layout == KeyboardLayout::Jis {
        // JIS: Muhenkan, shorter space, Henkan, Katakana
        row4.push(VisualKey { label: "\u{7121}", code: 94, width: w }); // 無 (Muhenkan)
        row4.push(VisualKey { label: "\u{2500}\u{2500}", code: 57, width: (w + 1) * 4 }); // space (shorter)
        row4.push(VisualKey { label: "\u{5909}", code: 92, width: w }); // 変 (Henkan)
        row4.push(VisualKey { label: "\u{30AB}", code: 93, width: w }); // カ (Katakana)
    } else {
        // ANSI/ISO: full-width space
        row4.push(VisualKey { label: "\u{2500}\u{2500}\u{2500}\u{2500}", code: 57, width: (w + 1) * 6 }); // ────
    }

    row4.push(VisualKey { label: "Alt", code: 100, width: w });
    row4.push(VisualKey { label: "Ctl", code: 97, width: w });

    [row0, row1, row2, row3, row4]
}

#[cfg(target_os = "linux")]
fn detect_linux_layout() -> KeyboardLayout {
    // Try setxkbmap first
    if let Ok(output) = std::process::Command::new("setxkbmap")
        .args(["-query"])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            return parse_xkb_query(&stdout);
        }
    }

    // Fallback: read /etc/default/keyboard
    if let Ok(contents) = std::fs::read_to_string("/etc/default/keyboard") {
        return parse_keyboard_config(&contents);
    }

    // Fallback: read localectl
    if let Ok(output) = std::process::Command::new("localectl")
        .args(["status"])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            return parse_localectl(&stdout);
        }
    }

    KeyboardLayout::Ansi
}

#[cfg(target_os = "linux")]
fn parse_xkb_query(output: &str) -> KeyboardLayout {
    // Format: "model:      pc105\nlayout:     gb\nvariant:    ..."
    let mut model = "";
    let mut layout = "";

    for line in output.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("model:") {
            model = value.trim();
        } else if let Some(value) = line.strip_prefix("layout:") {
            layout = value.trim();
            // Take only the first layout if comma-separated
            if let Some(first) = layout.split(',').next() {
                layout = first.trim();
            }
        }
    }

    // JIS detection
    if layout == "jp" || model.contains("jp") || model.contains("jis") {
        return KeyboardLayout::Jis;
    }

    // ISO detection: models with 102/105 keys, or known ISO layouts
    if model.contains("105") || model.contains("102") || model.contains("iso") {
        return KeyboardLayout::Iso;
    }

    // Known ISO layout countries
    const ISO_LAYOUTS: &[&str] = &[
        "gb", "uk", "de", "fr", "es", "it", "pt", "nl", "be", "dk", "fi",
        "no", "se", "ch", "at", "ie", "is", "cz", "sk", "hu", "pl", "ro",
        "bg", "hr", "si", "rs", "tr", "br", "latam",
    ];

    if ISO_LAYOUTS.contains(&layout) {
        return KeyboardLayout::Iso;
    }

    // Default to ANSI for "us" and anything else
    KeyboardLayout::Ansi
}

#[cfg(target_os = "linux")]
fn parse_keyboard_config(contents: &str) -> KeyboardLayout {
    // Format: XKBLAYOUT="gb"
    for line in contents.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("XKBLAYOUT=") {
            let layout = value.trim_matches('"');
            if layout == "jp" {
                return KeyboardLayout::Jis;
            }
            // Use same ISO check
            let first = layout.split(',').next().unwrap_or(layout);
            return parse_xkb_query(&format!("layout: {}", first));
        }
        if let Some(value) = line.strip_prefix("XKBMODEL=") {
            let model = value.trim_matches('"');
            if model.contains("jp") || model.contains("jis") {
                return KeyboardLayout::Jis;
            }
            if model.contains("105") || model.contains("102") {
                return KeyboardLayout::Iso;
            }
        }
    }
    KeyboardLayout::Ansi
}

#[cfg(target_os = "linux")]
fn parse_localectl(output: &str) -> KeyboardLayout {
    // Format: "X11 Layout: gb"
    for line in output.lines() {
        if let Some(value) = line.trim().strip_prefix("X11 Layout:") {
            let layout = value.trim();
            return parse_xkb_query(&format!("layout: {}", layout));
        }
        if let Some(value) = line.trim().strip_prefix("X11 Model:") {
            let model = value.trim();
            return parse_xkb_query(&format!("model: {}", model));
        }
    }
    KeyboardLayout::Ansi
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_ansi() {
        assert_eq!(KeyboardLayout::default(), KeyboardLayout::Ansi);
    }

    #[test]
    fn layout_names() {
        assert_eq!(KeyboardLayout::Ansi.name(), "ANSI (US)");
        assert_eq!(KeyboardLayout::Iso.name(), "ISO (EU)");
        assert_eq!(KeyboardLayout::Jis.name(), "JIS (JP)");
    }

    #[test]
    fn iso_has_extra_key() {
        assert!(!KeyboardLayout::Ansi.has_iso_key());
        assert!(KeyboardLayout::Iso.has_iso_key());
        assert!(!KeyboardLayout::Jis.has_iso_key());
    }

    #[test]
    fn jis_has_extra_keys() {
        assert!(!KeyboardLayout::Ansi.has_jis_keys());
        assert!(!KeyboardLayout::Iso.has_jis_keys());
        assert!(KeyboardLayout::Jis.has_jis_keys());
    }

    #[test]
    fn ansi_layout_rows_have_5_rows() {
        let rows = layout_rows(KeyboardLayout::Ansi);
        assert_eq!(rows.len(), 5);
    }

    #[test]
    fn iso_shift_row_has_extra_key() {
        let rows = layout_rows(KeyboardLayout::Iso);
        // Row 3 (shift) should have the extra ISO key (scancode 86)
        let has_iso_key = rows[3].iter().any(|k| k.code == 86);
        assert!(has_iso_key);
    }

    #[test]
    fn jis_bottom_row_has_henkan() {
        let rows = layout_rows(KeyboardLayout::Jis);
        // Row 4 (bottom) should have Henkan (scancode 92)
        let has_henkan = rows[4].iter().any(|k| k.code == 92);
        assert!(has_henkan);
    }

    #[test]
    fn ansi_no_iso_key() {
        let rows = layout_rows(KeyboardLayout::Ansi);
        let has_iso_key = rows[3].iter().any(|k| k.code == 86);
        assert!(!has_iso_key);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_xkb_us_is_ansi() {
        let output = "rules:      evdev\nmodel:      pc104\nlayout:     us\n";
        assert_eq!(parse_xkb_query(output), KeyboardLayout::Ansi);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_xkb_gb_is_iso() {
        let output = "rules:      evdev\nmodel:      pc105\nlayout:     gb\n";
        assert_eq!(parse_xkb_query(output), KeyboardLayout::Iso);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_xkb_jp_is_jis() {
        let output = "rules:      evdev\nmodel:      jp106\nlayout:     jp\n";
        assert_eq!(parse_xkb_query(output), KeyboardLayout::Jis);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_xkb_de_is_iso() {
        let output = "rules:      evdev\nmodel:      pc105\nlayout:     de\n";
        assert_eq!(parse_xkb_query(output), KeyboardLayout::Iso);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_keyboard_config_gb() {
        let contents = "XKBMODEL=\"pc105\"\nXKBLAYOUT=\"gb\"\n";
        assert_eq!(parse_keyboard_config(contents), KeyboardLayout::Iso);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_keyboard_config_jp() {
        let contents = "XKBMODEL=\"jp106\"\nXKBLAYOUT=\"jp\"\n";
        assert_eq!(parse_keyboard_config(contents), KeyboardLayout::Jis);
    }
}
