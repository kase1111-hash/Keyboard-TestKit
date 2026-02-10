//! Visual keyboard layout rendering

use crate::keyboard::{KeyCode, KeyboardState};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::Widget,
};

/// Sleek keyboard colors
mod palette {
    use ratatui::style::Color;
    pub const KEY_OFF: Color = Color::Rgb(40, 40, 50);
    pub const KEY_ON: Color = Color::Rgb(80, 200, 120);
    pub const KEY_USED: Color = Color::Rgb(55, 55, 70);
    pub const TEXT: Color = Color::Rgb(180, 180, 190);
    pub const TEXT_ON: Color = Color::Rgb(20, 20, 25);
}

/// Visual representation of a keyboard
pub struct KeyboardVisual<'a> {
    keyboard_state: &'a KeyboardState,
}

impl<'a> KeyboardVisual<'a> {
    pub fn new(keyboard_state: &'a KeyboardState) -> Self {
        Self { keyboard_state }
    }

    fn key_style(&self, code: KeyCode) -> (Color, Color, bool) {
        let pressed = self.keyboard_state.pressed_keys().contains(&code);
        if pressed {
            (palette::KEY_ON, palette::TEXT_ON, true)
        } else if self
            .keyboard_state
            .get_key_state(code)
            .is_some_and(|s| s.press_count > 0)
        {
            (palette::KEY_USED, palette::TEXT, false)
        } else {
            (palette::KEY_OFF, palette::TEXT, false)
        }
    }

    fn render_key(&self, buf: &mut Buffer, x: u16, y: u16, label: &str, code: KeyCode, w: u16) {
        let (bg, fg, bold) = self.key_style(code);
        let mut style = Style::default().fg(fg).bg(bg);
        if bold {
            style = style.add_modifier(Modifier::BOLD);
        }
        if y < buf.area.height && x + w <= buf.area.width {
            buf.set_string(x, y, format!("{:^w$}", label, w = w as usize), style);
        }
    }
}

impl<'a> Widget for KeyboardVisual<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 40 || area.height < 5 {
            buf.set_string(
                area.x,
                area.y,
                "⌨ Window too small",
                Style::default().fg(palette::TEXT),
            );
            return;
        }

        let w = 4u16;
        let x0 = area.x + 1;
        let y0 = area.y;

        // Row 0: Numbers
        let row0 = [
            ("`", 41),
            ("1", 2),
            ("2", 3),
            ("3", 4),
            ("4", 5),
            ("5", 6),
            ("6", 7),
            ("7", 8),
            ("8", 9),
            ("9", 10),
            ("0", 11),
            ("-", 12),
            ("=", 13),
        ];
        let mut x = x0;
        for (l, c) in row0 {
            self.render_key(buf, x, y0, l, KeyCode(c), w);
            x += w + 1;
        }
        self.render_key(buf, x, y0, "←", KeyCode(14), w + 2);

        // Row 1: QWERTY
        let row1 = [
            ("Q", 16),
            ("W", 17),
            ("E", 18),
            ("R", 19),
            ("T", 20),
            ("Y", 21),
            ("U", 22),
            ("I", 23),
            ("O", 24),
            ("P", 25),
            ("[", 26),
            ("]", 27),
            ("\\", 43),
        ];
        self.render_key(buf, x0, y0 + 1, "⇥", KeyCode(15), w);
        x = x0 + w + 1;
        for (l, c) in row1 {
            self.render_key(buf, x, y0 + 1, l, KeyCode(c), w);
            x += w + 1;
        }

        // Row 2: Home
        let row2 = [
            ("A", 30),
            ("S", 31),
            ("D", 32),
            ("F", 33),
            ("G", 34),
            ("H", 35),
            ("J", 36),
            ("K", 37),
            ("L", 38),
            (";", 39),
            ("'", 40),
        ];
        self.render_key(buf, x0, y0 + 2, "⇪", KeyCode(58), w + 1);
        x = x0 + w + 2;
        for (l, c) in row2 {
            self.render_key(buf, x, y0 + 2, l, KeyCode(c), w);
            x += w + 1;
        }
        self.render_key(buf, x, y0 + 2, "↵", KeyCode(28), w + 2);

        // Row 3: Shift
        let row3 = [
            ("Z", 44),
            ("X", 45),
            ("C", 46),
            ("V", 47),
            ("B", 48),
            ("N", 49),
            ("M", 50),
            (",", 51),
            (".", 52),
            ("/", 53),
        ];
        self.render_key(buf, x0, y0 + 3, "⇧", KeyCode(42), w + 2);
        x = x0 + w + 3;
        for (l, c) in row3 {
            self.render_key(buf, x, y0 + 3, l, KeyCode(c), w);
            x += w + 1;
        }
        self.render_key(buf, x, y0 + 3, "⇧", KeyCode(54), w + 3);

        // Row 4: Bottom
        self.render_key(buf, x0, y0 + 4, "Ctl", KeyCode(29), w);
        self.render_key(buf, x0 + w + 1, y0 + 4, "◆", KeyCode(125), w);
        self.render_key(buf, x0 + (w + 1) * 2, y0 + 4, "Alt", KeyCode(56), w);
        let sp_x = x0 + (w + 1) * 3;
        let sp_w = (w + 1) * 6;
        self.render_key(buf, sp_x, y0 + 4, "────", KeyCode(57), sp_w);
        let rx = sp_x + sp_w + 1;
        self.render_key(buf, rx, y0 + 4, "Alt", KeyCode(100), w);
        self.render_key(buf, rx + w + 1, y0 + 4, "Ctl", KeyCode(97), w);

        // Arrows
        if area.width > 70 {
            let ax = rx + (w + 1) * 3;
            self.render_key(buf, ax + w + 1, y0 + 3, "▲", KeyCode(103), w);
            self.render_key(buf, ax, y0 + 4, "◀", KeyCode(105), w);
            self.render_key(buf, ax + w + 1, y0 + 4, "▼", KeyCode(108), w);
            self.render_key(buf, ax + (w + 1) * 2, y0 + 4, "▶", KeyCode(106), w);
        }
    }
}
