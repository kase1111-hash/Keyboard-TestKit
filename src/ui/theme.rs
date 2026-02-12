//! Theme color definitions for the UI
//!
//! Provides dark and light color palettes that can be switched at runtime.

use crate::config::Theme;
use ratatui::style::Color;

/// Complete color palette for the UI
#[derive(Debug, Clone, Copy)]
pub struct ThemeColors {
    /// Main background
    pub bg: Color,
    /// Primary foreground text
    pub fg: Color,
    /// Dimmed/secondary text
    pub dim: Color,
    /// Accent color (headings, active tab)
    pub cyan: Color,
    /// Success / OK status
    pub green: Color,
    /// Warning status
    pub yellow: Color,
    /// Error status
    pub red: Color,
    /// Key idle (unpressed) background
    pub key_off: Color,
    /// Key pressed background
    pub key_on: Color,
    /// Key previously used background
    pub key_used: Color,
    /// Key label text (idle)
    pub key_text: Color,
    /// Key label text (pressed)
    pub key_text_on: Color,
}

impl ThemeColors {
    /// Create a color palette for the given theme variant
    pub fn from_theme(theme: Theme) -> Self {
        match theme {
            Theme::Dark => Self::dark(),
            Theme::Light => Self::light(),
        }
    }

    /// Dark theme - original color scheme
    pub fn dark() -> Self {
        Self {
            bg: Color::Rgb(22, 22, 30),
            fg: Color::Rgb(200, 200, 210),
            dim: Color::Rgb(90, 90, 110),
            cyan: Color::Rgb(80, 200, 220),
            green: Color::Rgb(80, 200, 120),
            yellow: Color::Rgb(240, 180, 80),
            red: Color::Rgb(240, 90, 100),
            key_off: Color::Rgb(40, 40, 50),
            key_on: Color::Rgb(80, 200, 120),
            key_used: Color::Rgb(55, 55, 70),
            key_text: Color::Rgb(180, 180, 190),
            key_text_on: Color::Rgb(20, 20, 25),
        }
    }

    /// Light theme - high contrast for bright terminals
    pub fn light() -> Self {
        Self {
            bg: Color::Rgb(245, 245, 248),
            fg: Color::Rgb(30, 30, 40),
            dim: Color::Rgb(130, 130, 150),
            cyan: Color::Rgb(0, 130, 160),
            green: Color::Rgb(30, 150, 70),
            yellow: Color::Rgb(180, 120, 0),
            red: Color::Rgb(200, 50, 60),
            key_off: Color::Rgb(220, 220, 228),
            key_on: Color::Rgb(30, 150, 70),
            key_used: Color::Rgb(200, 200, 212),
            key_text: Color::Rgb(50, 50, 60),
            key_text_on: Color::Rgb(255, 255, 255),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_theme_creates_palette() {
        let colors = ThemeColors::dark();
        assert_eq!(colors.bg, Color::Rgb(22, 22, 30));
        assert_eq!(colors.green, Color::Rgb(80, 200, 120));
    }

    #[test]
    fn light_theme_creates_palette() {
        let colors = ThemeColors::light();
        assert_eq!(colors.bg, Color::Rgb(245, 245, 248));
        assert_eq!(colors.green, Color::Rgb(30, 150, 70));
    }

    #[test]
    fn from_theme_selects_correct_palette() {
        let dark = ThemeColors::from_theme(Theme::Dark);
        let light = ThemeColors::from_theme(Theme::Light);

        // Dark and light should have different backgrounds
        assert_ne!(dark.bg, light.bg);
    }
}
