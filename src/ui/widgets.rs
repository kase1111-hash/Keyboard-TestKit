//! Custom TUI widgets

use super::theme::ThemeColors;
use crate::tests::{ResultStatus, TestResult};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Borders, Widget},
};

/// Widget for displaying test results
pub struct ResultsPanel<'a> {
    results: &'a [TestResult],
    title: &'a str,
    colors: ThemeColors,
}

impl<'a> ResultsPanel<'a> {
    pub fn new(results: &'a [TestResult], title: &'a str) -> Self {
        Self {
            results,
            title,
            colors: ThemeColors::dark(),
        }
    }

    pub fn theme(mut self, colors: ThemeColors) -> Self {
        self.colors = colors;
        self
    }

    fn status_style(&self, status: ResultStatus) -> (Color, &'static str) {
        match status {
            ResultStatus::Ok => (self.colors.green, "✓"),
            ResultStatus::Warning => (self.colors.yellow, "!"),
            ResultStatus::Error => (self.colors.red, "✗"),
            ResultStatus::Info => (self.colors.dim, "·"),
        }
    }
}

impl<'a> Widget for ResultsPanel<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(format!(" {} ", self.title))
            .borders(Borders::ALL)
            .border_set(border::ROUNDED)
            .border_style(Style::default().fg(self.colors.dim));

        let inner = block.inner(area);
        block.render(area, buf);

        let mut y = inner.y;
        for result in self.results {
            if y >= inner.y + inner.height {
                break;
            }

            let is_header = result.label.starts_with("---") || result.label.starts_with("===");

            if is_header {
                let text = result.label.trim_matches('-').trim_matches('=').trim();
                buf.set_string(
                    inner.x + 1,
                    y,
                    text,
                    Style::default()
                        .fg(self.colors.cyan)
                        .add_modifier(Modifier::BOLD),
                );
            } else if !result.label.is_empty() || !result.value.is_empty() {
                let (color, sym) = self.status_style(result.status);
                let line = Line::from(vec![
                    Span::styled(format!(" {} ", sym), Style::default().fg(color)),
                    Span::styled(
                        format!("{:<18}", result.label),
                        Style::default().fg(self.colors.fg),
                    ),
                    Span::styled(
                        &result.value,
                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                    ),
                ]);
                buf.set_line(inner.x, y, &line, inner.width);
            }
            y += 1;
        }
    }
}

/// Widget for the help screen
pub struct HelpPanel {
    colors: ThemeColors,
}

impl HelpPanel {
    pub fn new() -> Self {
        Self {
            colors: ThemeColors::dark(),
        }
    }

    pub fn theme(mut self, colors: ThemeColors) -> Self {
        self.colors = colors;
        self
    }
}

impl Default for HelpPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for HelpPanel {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(" ⌨ Help ")
            .borders(Borders::ALL)
            .border_set(border::ROUNDED)
            .border_style(Style::default().fg(self.colors.cyan));

        let inner = block.inner(area);
        block.render(area, buf);

        let logo = [
            ("", self.colors.cyan),
            ("  ╭───────────────────────────────╮", self.colors.cyan),
            ("  │  ⌨  KEYBOARD TESTKIT  v0.1   │", self.colors.cyan),
            ("  ╰───────────────────────────────╯", self.colors.cyan),
            ("", self.colors.dim),
        ];

        let mut y = inner.y;
        for (line, color) in &logo {
            if y < inner.y + inner.height {
                buf.set_string(
                    inner.x,
                    y,
                    line,
                    Style::default().fg(*color).add_modifier(Modifier::BOLD),
                );
                y += 1;
            }
        }

        let sections = [
            (
                "NAV",
                &[
                    ("Tab", "Switch view"),
                    ("1-0", "Jump to view"),
                    ("m", "Toggle shortcuts"),
                    ("q", "Quit"),
                ][..],
            ),
            (
                "CTL",
                &[
                    ("Space", "Pause"),
                    ("r/R", "Reset"),
                    ("e", "Export"),
                    ("t", "Toggle theme"),
                    ("?", "Help"),
                ][..],
            ),
            (
                "TESTS",
                &[
                    ("1", "Dashboard"),
                    ("2", "Polling"),
                    ("3", "Bounce"),
                    ("4", "Sticky"),
                    ("5", "NKRO"),
                    ("6", "Latency"),
                    ("7", "Shortcuts"),
                    ("8", "Virtual"),
                    ("9", "OEM/FN"),
                    ("0", "Help"),
                ][..],
            ),
            (
                "OEM",
                &[
                    ("a", "Add FN scancode"),
                    ("f", "Cycle FN mode"),
                    ("c", "Clear mappings"),
                ][..],
            ),
        ];

        for (header, items) in &sections {
            if y >= inner.y + inner.height {
                break;
            }
            buf.set_string(
                inner.x + 2,
                y,
                *header,
                Style::default()
                    .fg(self.colors.cyan)
                    .add_modifier(Modifier::BOLD),
            );
            y += 1;

            for (key, desc) in *items {
                if y >= inner.y + inner.height {
                    break;
                }
                let line = Line::from(vec![
                    Span::styled(
                        format!("  {:<8}", key),
                        Style::default().fg(self.colors.yellow),
                    ),
                    Span::styled(*desc, Style::default().fg(self.colors.dim)),
                ]);
                buf.set_line(inner.x, y, &line, inner.width);
                y += 1;
            }
            y += 1; // Gap between sections
        }
    }
}

/// Status bar widget
pub struct StatusBar<'a> {
    state: &'a str,
    view: &'a str,
    elapsed: &'a str,
    events: u64,
    message: Option<&'a str>,
    colors: ThemeColors,
}

impl<'a> StatusBar<'a> {
    pub fn new(state: &'a str, view: &'a str, elapsed: &'a str, events: u64) -> Self {
        Self {
            state,
            view,
            elapsed,
            events,
            message: None,
            colors: ThemeColors::dark(),
        }
    }

    pub fn message(mut self, message: Option<&'a str>) -> Self {
        self.message = message;
        self
    }

    pub fn theme(mut self, colors: ThemeColors) -> Self {
        self.colors = colors;
        self
    }
}

impl<'a> Widget for StatusBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let bg = Style::default().bg(self.colors.bg).fg(self.colors.fg);
        for x in area.x..area.x + area.width {
            buf.set_string(x, area.y, " ", bg);
        }

        let (icon, color) = if self.state == "RUNNING" {
            ("▶", self.colors.green)
        } else {
            ("▪", self.colors.yellow)
        };

        buf.set_string(
            area.x + 1,
            area.y,
            icon,
            Style::default().bg(self.colors.bg).fg(color),
        );

        let left = format!(" {} │ {}", self.state, self.view);
        buf.set_string(area.x + 2, area.y, &left, bg.add_modifier(Modifier::BOLD));

        if let Some(msg) = self.message {
            let x = area.x + (area.width / 2).saturating_sub(msg.len() as u16 / 2);
            buf.set_string(
                x,
                area.y,
                msg,
                Style::default().bg(self.colors.bg).fg(self.colors.yellow),
            );
        }

        let right = format!("{} │ {} ", self.elapsed, self.events);
        let x = area.x + area.width.saturating_sub(right.len() as u16);
        buf.set_string(x, area.y, &right, bg);
    }
}

/// Tab bar widget
pub struct TabBar<'a> {
    tabs: &'a [&'a str],
    selected: usize,
    colors: ThemeColors,
}

impl<'a> TabBar<'a> {
    pub fn new(tabs: &'a [&'a str], selected: usize) -> Self {
        Self {
            tabs,
            selected,
            colors: ThemeColors::dark(),
        }
    }

    pub fn theme(mut self, colors: ThemeColors) -> Self {
        self.colors = colors;
        self
    }
}

impl<'a> Widget for TabBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let bg = Style::default().bg(self.colors.bg);
        for x in area.x..area.x + area.width {
            buf.set_string(x, area.y, " ", bg);
        }

        let mut x = area.x + 1;
        for (i, tab) in self.tabs.iter().enumerate() {
            let sel = i == self.selected;
            let style = if sel {
                Style::default()
                    .fg(self.colors.cyan)
                    .bg(self.colors.bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(self.colors.dim).bg(self.colors.bg)
            };

            let num_style = if sel {
                Style::default()
                    .fg(self.colors.yellow)
                    .bg(self.colors.bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(self.colors.dim).bg(self.colors.bg)
            };

            let label = format!("{}.{}", i + 1, tab);
            if x + label.len() as u16 + 2 <= area.x + area.width {
                buf.set_string(x, area.y, format!("{}", i + 1), num_style);
                buf.set_string(x + 1, area.y, format!(".{} ", tab), style);
                x += label.len() as u16 + 2;
            }
        }
    }
}

/// Settings panel widget for in-app configuration
pub struct SettingsPanel<'a> {
    items: &'a [SettingsItem],
    selected: usize,
    colors: ThemeColors,
}

/// A single setting in the settings panel
pub struct SettingsItem {
    pub label: String,
    pub value: String,
    pub editable: bool,
}

impl<'a> SettingsPanel<'a> {
    pub fn new(items: &'a [SettingsItem], selected: usize) -> Self {
        Self {
            items,
            selected,
            colors: ThemeColors::dark(),
        }
    }

    pub fn theme(mut self, colors: ThemeColors) -> Self {
        self.colors = colors;
        self
    }
}

impl<'a> Widget for SettingsPanel<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(" ⚙ Settings ")
            .borders(Borders::ALL)
            .border_set(border::ROUNDED)
            .border_style(Style::default().fg(self.colors.cyan));

        let inner = block.inner(area);
        block.render(area, buf);

        // Header
        let mut y = inner.y;
        buf.set_string(
            inner.x + 2,
            y,
            "Use ↑↓ to select, ←→ to adjust, s to save",
            Style::default().fg(self.colors.dim),
        );
        y += 2;

        for (i, item) in self.items.iter().enumerate() {
            if y >= inner.y + inner.height {
                break;
            }

            let is_selected = i == self.selected;
            let (label_style, value_style) = if is_selected {
                (
                    Style::default()
                        .fg(self.colors.cyan)
                        .add_modifier(Modifier::BOLD),
                    Style::default()
                        .fg(self.colors.yellow)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                (
                    Style::default().fg(self.colors.fg),
                    Style::default().fg(self.colors.dim),
                )
            };

            let cursor = if is_selected { "▸ " } else { "  " };
            let cursor_style = Style::default().fg(self.colors.cyan);

            let line = Line::from(vec![
                Span::styled(cursor, cursor_style),
                Span::styled(format!("{:<30}", item.label), label_style),
                Span::styled(&item.value, value_style),
            ]);
            buf.set_line(inner.x, y, &line, inner.width);
            y += 1;
        }

        // Footer
        if y + 2 < inner.y + inner.height {
            y += 1;
            buf.set_string(
                inner.x + 2,
                y,
                "s = Save to config file",
                Style::default().fg(self.colors.dim),
            );
        }
    }
}

/// Shortcut warning overlay - displayed in any view when a shortcut is detected
pub struct ShortcutOverlay<'a> {
    combo: &'a str,
    description: Option<&'a str>,
    colors: ThemeColors,
}

impl<'a> ShortcutOverlay<'a> {
    pub fn new(combo: &'a str) -> Self {
        Self {
            combo,
            description: None,
            colors: ThemeColors::dark(),
        }
    }

    pub fn description(mut self, desc: Option<&'a str>) -> Self {
        self.description = desc;
        self
    }

    pub fn theme(mut self, colors: ThemeColors) -> Self {
        self.colors = colors;
        self
    }
}

impl<'a> Widget for ShortcutOverlay<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Render as a floating box in the top-right
        let width = (self.combo.len() as u16 + 8).max(20).min(area.width);
        let height = if self.description.is_some() { 3 } else { 2 };

        if area.width < width + 2 || area.height < height {
            return;
        }

        let x = area.x + area.width - width - 1;
        let y = area.y;

        // Background
        let bg_style = Style::default()
            .bg(self.colors.bg)
            .fg(self.colors.yellow);
        for dy in 0..height {
            for dx in 0..width {
                if x + dx < area.x + area.width && y + dy < area.y + area.height {
                    buf.set_string(x + dx, y + dy, " ", bg_style);
                }
            }
        }

        // Border top
        buf.set_string(
            x,
            y,
            format!("╭{}╮", "─".repeat((width - 2) as usize)),
            Style::default().fg(self.colors.yellow).bg(self.colors.bg),
        );

        // Content
        let content = format!(" ⚡ {} ", self.combo);
        buf.set_string(
            x + 1,
            y + 1,
            &content,
            Style::default()
                .fg(self.colors.yellow)
                .bg(self.colors.bg)
                .add_modifier(Modifier::BOLD),
        );

        if let Some(desc) = self.description {
            buf.set_string(
                x + 1,
                y + 2,
                format!(" {} ", desc),
                Style::default().fg(self.colors.dim).bg(self.colors.bg),
            );
        }
    }
}
