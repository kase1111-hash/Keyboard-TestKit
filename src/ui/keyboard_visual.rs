//! Visual keyboard layout rendering

use crate::keyboard::{KeyCode, KeyboardState};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::Widget,
};

/// Modern color palette for keyboard
mod palette {
    use ratatui::style::Color;

    pub const KEY_INACTIVE: Color = Color::Rgb(50, 50, 60);
    pub const KEY_PRESSED: Color = Color::Rgb(100, 220, 140);
    pub const KEY_RECENTLY_USED: Color = Color::Rgb(70, 70, 85);
    pub const KEY_TEXT_LIGHT: Color = Color::Rgb(200, 200, 210);
    pub const KEY_TEXT_DARK: Color = Color::Rgb(20, 20, 30);
}

/// Visual representation of a keyboard
pub struct KeyboardVisual<'a> {
    keyboard_state: &'a KeyboardState,
    compact: bool,
}

impl<'a> KeyboardVisual<'a> {
    pub fn new(keyboard_state: &'a KeyboardState) -> Self {
        Self {
            keyboard_state,
            compact: false,
        }
    }

    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }

    /// Get color for a key based on its state
    fn key_style(&self, key_code: KeyCode) -> (Color, Color, bool) {
        let pressed = self.keyboard_state.pressed_keys().contains(&key_code);
        let key_state = self.keyboard_state.get_key_state(key_code);

        if pressed {
            (palette::KEY_PRESSED, palette::KEY_TEXT_DARK, true)
        } else if let Some(state) = key_state {
            if state.press_count > 0 {
                (palette::KEY_RECENTLY_USED, palette::KEY_TEXT_LIGHT, false)
            } else {
                (palette::KEY_INACTIVE, palette::KEY_TEXT_LIGHT, false)
            }
        } else {
            (palette::KEY_INACTIVE, palette::KEY_TEXT_LIGHT, false)
        }
    }

    /// Render a single key with modern styling
    fn render_key(&self, buf: &mut Buffer, x: u16, y: u16, key: &str, key_code: KeyCode, width: u16) {
        let (bg_color, fg_color, is_pressed) = self.key_style(key_code);

        let mut style = Style::default().fg(fg_color).bg(bg_color);
        if is_pressed {
            style = style.add_modifier(Modifier::BOLD);
        }

        // Draw key with centered label
        let key_str = format!("{:^width$}", key, width = width as usize);
        if y < buf.area.height && x + width <= buf.area.width {
            buf.set_string(x, y, &key_str, style);
        }
    }
}

impl<'a> Widget for KeyboardVisual<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 40 || area.height < 5 {
            let msg = "⌨ Terminal too small for keyboard view";
            let style = Style::default().fg(Color::Rgb(150, 150, 170));
            buf.set_string(area.x, area.y, msg, style);
            return;
        }

        let key_width = if self.compact { 3 } else { 4 };
        let start_x = area.x + 1;
        let start_y = area.y;

        // Row 1: Number row
        let row1_keys = [
            ("`", KeyCode(41)), ("1", KeyCode(2)), ("2", KeyCode(3)), ("3", KeyCode(4)),
            ("4", KeyCode(5)), ("5", KeyCode(6)), ("6", KeyCode(7)), ("7", KeyCode(8)),
            ("8", KeyCode(9)), ("9", KeyCode(10)), ("0", KeyCode(11)), ("-", KeyCode(12)),
            ("=", KeyCode(13)),
        ];

        let mut x = start_x;
        for (label, code) in &row1_keys {
            self.render_key(buf, x, start_y, label, *code, key_width);
            x += key_width + 1;
        }
        // Backspace (wider)
        self.render_key(buf, x, start_y, "←", KeyCode(14), key_width + 2);

        // Row 2: QWERTY row
        let row2_keys = [
            ("Q", KeyCode(16)), ("W", KeyCode(17)), ("E", KeyCode(18)), ("R", KeyCode(19)),
            ("T", KeyCode(20)), ("Y", KeyCode(21)), ("U", KeyCode(22)), ("I", KeyCode(23)),
            ("O", KeyCode(24)), ("P", KeyCode(25)), ("[", KeyCode(26)), ("]", KeyCode(27)),
            ("\\", KeyCode(43)),
        ];

        self.render_key(buf, start_x, start_y + 1, "⇥", KeyCode(15), key_width);
        x = start_x + key_width + 1;
        for (label, code) in &row2_keys {
            self.render_key(buf, x, start_y + 1, label, *code, key_width);
            x += key_width + 1;
        }

        // Row 3: Home row
        let row3_keys = [
            ("A", KeyCode(30)), ("S", KeyCode(31)), ("D", KeyCode(32)), ("F", KeyCode(33)),
            ("G", KeyCode(34)), ("H", KeyCode(35)), ("J", KeyCode(36)), ("K", KeyCode(37)),
            ("L", KeyCode(38)), (";", KeyCode(39)), ("'", KeyCode(40)),
        ];

        self.render_key(buf, start_x, start_y + 2, "⇪", KeyCode(58), key_width + 1);
        x = start_x + key_width + 2;
        for (label, code) in &row3_keys {
            self.render_key(buf, x, start_y + 2, label, *code, key_width);
            x += key_width + 1;
        }
        self.render_key(buf, x, start_y + 2, "↵", KeyCode(28), key_width + 2);

        // Row 4: Shift row
        let row4_keys = [
            ("Z", KeyCode(44)), ("X", KeyCode(45)), ("C", KeyCode(46)), ("V", KeyCode(47)),
            ("B", KeyCode(48)), ("N", KeyCode(49)), ("M", KeyCode(50)), (",", KeyCode(51)),
            (".", KeyCode(52)), ("/", KeyCode(53)),
        ];

        self.render_key(buf, start_x, start_y + 3, "⇧", KeyCode(42), key_width + 2);
        x = start_x + key_width + 3;
        for (label, code) in &row4_keys {
            self.render_key(buf, x, start_y + 3, label, *code, key_width);
            x += key_width + 1;
        }
        self.render_key(buf, x, start_y + 3, "⇧", KeyCode(54), key_width + 3);

        // Row 5: Bottom row
        self.render_key(buf, start_x, start_y + 4, "Ctl", KeyCode(29), key_width);
        self.render_key(buf, start_x + key_width + 1, start_y + 4, "⊞", KeyCode(125), key_width);
        self.render_key(buf, start_x + (key_width + 1) * 2, start_y + 4, "Alt", KeyCode(56), key_width);

        // Spacebar
        let space_start = start_x + (key_width + 1) * 3;
        let space_width = (key_width + 1) * 6;
        self.render_key(buf, space_start, start_y + 4, "━━━━━━", KeyCode(57), space_width);

        // Right side modifiers
        let right_start = space_start + space_width + 1;
        self.render_key(buf, right_start, start_y + 4, "Alt", KeyCode(100), key_width);
        self.render_key(buf, right_start + key_width + 1, start_y + 4, "Ctl", KeyCode(97), key_width);

        // Arrow keys (if space permits)
        if area.width > 70 {
            let arrow_x = right_start + (key_width + 1) * 3;
            self.render_key(buf, arrow_x + key_width + 1, start_y + 3, "▲", KeyCode(103), key_width);
            self.render_key(buf, arrow_x, start_y + 4, "◀", KeyCode(105), key_width);
            self.render_key(buf, arrow_x + key_width + 1, start_y + 4, "▼", KeyCode(108), key_width);
            self.render_key(buf, arrow_x + (key_width + 1) * 2, start_y + 4, "▶", KeyCode(106), key_width);
        }
    }
}
