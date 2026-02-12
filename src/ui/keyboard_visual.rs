//! Visual keyboard layout rendering

use super::theme::ThemeColors;
use crate::keyboard::layout::{layout_rows, KeyboardLayout};
use crate::keyboard::{KeyCode, KeyboardState};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::Widget,
};

/// Visual representation of a keyboard
pub struct KeyboardVisual<'a> {
    keyboard_state: &'a KeyboardState,
    colors: ThemeColors,
    layout: KeyboardLayout,
}

impl<'a> KeyboardVisual<'a> {
    pub fn new(keyboard_state: &'a KeyboardState) -> Self {
        Self {
            keyboard_state,
            colors: ThemeColors::dark(),
            layout: KeyboardLayout::Ansi,
        }
    }

    pub fn theme(mut self, colors: ThemeColors) -> Self {
        self.colors = colors;
        self
    }

    pub fn layout(mut self, layout: KeyboardLayout) -> Self {
        self.layout = layout;
        self
    }

    fn key_style(&self, code: KeyCode) -> (Color, Color, bool) {
        let pressed = self.keyboard_state.pressed_keys().contains(&code);
        if pressed {
            (self.colors.key_on, self.colors.key_text_on, true)
        } else if self
            .keyboard_state
            .get_key_state(code)
            .is_some_and(|s| s.press_count > 0)
        {
            (self.colors.key_used, self.colors.key_text, false)
        } else {
            (self.colors.key_off, self.colors.key_text, false)
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
                "\u{2328} Window too small",
                Style::default().fg(self.colors.key_text),
            );
            return;
        }

        let rows = layout_rows(self.layout);
        let x0 = area.x + 1;
        let y0 = area.y;

        for (row_idx, row) in rows.iter().enumerate() {
            let y = y0 + row_idx as u16;
            let mut x = x0;

            for key in row {
                self.render_key(buf, x, y, key.label, KeyCode(key.code), key.width);
                x += key.width + 1;
            }
        }

        // Arrow keys (rendered separately, offset from main block)
        if area.width > 70 {
            let w = 4u16;
            // Position arrows after the bottom row
            let ax = x0 + 68;
            self.render_key(buf, ax + w + 1, y0 + 3, "\u{25B2}", KeyCode(103), w); // ▲
            self.render_key(buf, ax, y0 + 4, "\u{25C0}", KeyCode(105), w); // ◀
            self.render_key(buf, ax + w + 1, y0 + 4, "\u{25BC}", KeyCode(108), w); // ▼
            self.render_key(buf, ax + (w + 1) * 2, y0 + 4, "\u{25B6}", KeyCode(106), w); // ▶
        }
    }
}
