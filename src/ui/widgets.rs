//! Custom TUI widgets

use crate::tests::{TestResult, ResultStatus};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Widget},
};

/// Modern color palette
mod palette {
    use ratatui::style::Color;

    pub const BG_DARK: Color = Color::Rgb(30, 30, 40);
    pub const BG_MEDIUM: Color = Color::Rgb(45, 45, 55);
    pub const FG_PRIMARY: Color = Color::Rgb(220, 220, 230);
    pub const FG_SECONDARY: Color = Color::Rgb(150, 150, 170);
    pub const FG_MUTED: Color = Color::Rgb(100, 100, 120);
    pub const ACCENT_BLUE: Color = Color::Rgb(100, 150, 255);
    pub const ACCENT_GREEN: Color = Color::Rgb(100, 220, 140);
    pub const ACCENT_YELLOW: Color = Color::Rgb(255, 200, 100);
    pub const ACCENT_RED: Color = Color::Rgb(255, 100, 120);
    pub const ACCENT_CYAN: Color = Color::Rgb(100, 220, 230);
}

/// Widget for displaying test results
pub struct ResultsPanel<'a> {
    results: &'a [TestResult],
    title: &'a str,
}

impl<'a> ResultsPanel<'a> {
    pub fn new(results: &'a [TestResult], title: &'a str) -> Self {
        Self { results, title }
    }

    fn status_color(status: ResultStatus) -> Color {
        match status {
            ResultStatus::Ok => palette::ACCENT_GREEN,
            ResultStatus::Warning => palette::ACCENT_YELLOW,
            ResultStatus::Error => palette::ACCENT_RED,
            ResultStatus::Info => palette::ACCENT_CYAN,
        }
    }

    fn status_symbol(status: ResultStatus) -> &'static str {
        match status {
            ResultStatus::Ok => "●",
            ResultStatus::Warning => "◐",
            ResultStatus::Error => "○",
            ResultStatus::Info => "◦",
        }
    }
}

impl<'a> Widget for ResultsPanel<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(format!(" {} ", self.title))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette::FG_MUTED));

        let inner = block.inner(area);
        block.render(area, buf);

        let mut y = inner.y;
        for result in self.results {
            if y >= inner.y + inner.height {
                break;
            }

            let color = Self::status_color(result.status);
            let symbol = Self::status_symbol(result.status);

            // Check if this is a section header
            let is_header = result.label.starts_with("---") || result.label.starts_with("===");

            if is_header {
                // Render section headers with special styling
                let header_text = result.label.trim_matches('-').trim_matches('=').trim();
                let line = Line::from(vec![
                    Span::styled(
                        format!("  {} ", header_text),
                        Style::default()
                            .fg(palette::ACCENT_BLUE)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]);
                buf.set_line(inner.x, y, &line, inner.width);
            } else if result.label.is_empty() && result.value.is_empty() {
                // Empty line - just skip
            } else {
                let line = Line::from(vec![
                    Span::styled(
                        format!(" {} ", symbol),
                        Style::default().fg(color),
                    ),
                    Span::styled(
                        format!("{:<20}", result.label),
                        Style::default().fg(palette::FG_PRIMARY),
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
pub struct HelpPanel;

impl Widget for HelpPanel {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(" ⌨ Keyboard TestKit Help ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette::ACCENT_CYAN));

        let inner = block.inner(area);
        block.render(area, buf);

        let help_sections = vec![
            ("", ""),
            ("  NAVIGATION", "header"),
            ("", ""),
            ("  Tab / Shift+Tab", "Switch between views"),
            ("  1-8", "Jump to specific view"),
            ("  q / Esc", "Quit application"),
            ("", ""),
            ("  CONTROLS", "header"),
            ("", ""),
            ("  Space", "Pause/Resume testing"),
            ("  r", "Reset current test"),
            ("  R", "Reset all tests"),
            ("  e", "Export report to JSON"),
            ("  v", "Send virtual keys (Virtual view)"),
            ("  ?", "Show this help"),
            ("", ""),
            ("  TEST VIEWS", "header"),
            ("", ""),
            ("  1. Dashboard", "Overview of all tests"),
            ("  2. Polling", "Polling rate & jitter"),
            ("  3. Hold", "Bounce detection & timing"),
            ("  4. Sticky", "Stuck key detection"),
            ("  5. NKRO", "N-key rollover & ghosting"),
            ("  6. Latency", "Per-key input latency"),
            ("  7. Shortcuts", "Shortcut conflicts"),
            ("  8. Virtual", "Virtual input detection"),
            ("", ""),
            ("  Press any key to begin testing", "hint"),
        ];

        for (i, (left, right)) in help_sections.iter().enumerate() {
            if i as u16 >= inner.height {
                break;
            }

            let line = if *right == "header" {
                Line::from(vec![
                    Span::styled(
                        *left,
                        Style::default()
                            .fg(palette::ACCENT_BLUE)
                            .add_modifier(Modifier::BOLD),
                    ),
                ])
            } else if *right == "hint" {
                Line::from(vec![
                    Span::styled(
                        *left,
                        Style::default()
                            .fg(palette::FG_MUTED)
                            .add_modifier(Modifier::ITALIC),
                    ),
                ])
            } else if left.is_empty() {
                Line::from("")
            } else {
                Line::from(vec![
                    Span::styled(
                        format!("{:<22}", left),
                        Style::default()
                            .fg(palette::ACCENT_YELLOW)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        *right,
                        Style::default().fg(palette::FG_SECONDARY),
                    ),
                ])
            };

            buf.set_line(inner.x, inner.y + i as u16, &line, inner.width);
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
}

impl<'a> StatusBar<'a> {
    pub fn new(state: &'a str, view: &'a str, elapsed: &'a str, events: u64) -> Self {
        Self {
            state,
            view,
            elapsed,
            events,
            message: None,
        }
    }

    pub fn message(mut self, message: Option<&'a str>) -> Self {
        self.message = message;
        self
    }
}

impl<'a> Widget for StatusBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Background
        let bg_style = Style::default().bg(palette::BG_MEDIUM).fg(palette::FG_PRIMARY);
        for x in area.x..area.x + area.width {
            buf.set_string(x, area.y, " ", bg_style);
        }

        // Left side: state indicator
        let state_color = if self.state == "RUNNING" {
            palette::ACCENT_GREEN
        } else if self.state == "PAUSED" {
            palette::ACCENT_YELLOW
        } else {
            palette::ACCENT_RED
        };

        let state_indicator = if self.state == "RUNNING" { "▶" } else { "⏸" };
        buf.set_string(
            area.x + 1,
            area.y,
            state_indicator,
            Style::default().bg(palette::BG_MEDIUM).fg(state_color),
        );

        let left = format!(" {} │ {} ", self.state, self.view);
        buf.set_string(
            area.x + 3,
            area.y,
            &left,
            bg_style.add_modifier(Modifier::BOLD),
        );

        // Center: message if any
        if let Some(msg) = self.message {
            let msg_style = Style::default()
                .bg(palette::BG_MEDIUM)
                .fg(palette::ACCENT_YELLOW)
                .add_modifier(Modifier::ITALIC);
            let msg_x = area.x + (area.width / 2).saturating_sub(msg.len() as u16 / 2);
            buf.set_string(msg_x, area.y, msg, msg_style);
        }

        // Right side: elapsed time and events
        let right = format!("⏱ {} │ ⚡ {} ", self.elapsed, self.events);
        let right_x = area.x + area.width.saturating_sub(right.len() as u16);
        buf.set_string(right_x, area.y, &right, bg_style);
    }
}

/// Tab bar widget
pub struct TabBar<'a> {
    tabs: &'a [&'a str],
    selected: usize,
}

impl<'a> TabBar<'a> {
    pub fn new(tabs: &'a [&'a str], selected: usize) -> Self {
        Self { tabs, selected }
    }
}

impl<'a> Widget for TabBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Fill background
        let bg_style = Style::default().bg(palette::BG_DARK);
        for x in area.x..area.x + area.width {
            buf.set_string(x, area.y, " ", bg_style);
        }

        let mut x = area.x;

        for (i, tab) in self.tabs.iter().enumerate() {
            let is_selected = i == self.selected;

            let style = if is_selected {
                Style::default()
                    .fg(palette::ACCENT_CYAN)
                    .bg(palette::BG_DARK)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                Style::default()
                    .fg(palette::FG_SECONDARY)
                    .bg(palette::BG_DARK)
            };

            // Tab number
            let num_style = if is_selected {
                Style::default()
                    .fg(palette::ACCENT_YELLOW)
                    .bg(palette::BG_DARK)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(palette::FG_MUTED)
                    .bg(palette::BG_DARK)
            };

            let num = format!("{}", i + 1);
            let label = format!(".{} ", tab);
            let total_width = num.len() as u16 + label.len() as u16 + 1;

            if x + total_width <= area.x + area.width {
                buf.set_string(x, area.y, " ", bg_style);
                x += 1;
                buf.set_string(x, area.y, &num, num_style);
                x += num.len() as u16;
                buf.set_string(x, area.y, &label, style);
                x += label.len() as u16;

                // Separator dot
                if i < self.tabs.len() - 1 && x < area.x + area.width {
                    buf.set_string(
                        x,
                        area.y,
                        "·",
                        Style::default().fg(palette::FG_MUTED).bg(palette::BG_DARK),
                    );
                    x += 1;
                }
            }
        }
    }
}
