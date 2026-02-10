//! Custom TUI widgets

use crate::tests::{ResultStatus, TestResult};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Borders, Widget},
};

/// Sleek color palette - minimal, high contrast
mod palette {
    use ratatui::style::Color;
    pub const BG: Color = Color::Rgb(22, 22, 30);
    pub const FG: Color = Color::Rgb(200, 200, 210);
    pub const DIM: Color = Color::Rgb(90, 90, 110);
    pub const CYAN: Color = Color::Rgb(80, 200, 220);
    pub const GREEN: Color = Color::Rgb(80, 200, 120);
    pub const YELLOW: Color = Color::Rgb(240, 180, 80);
    pub const RED: Color = Color::Rgb(240, 90, 100);
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

    fn status_style(status: ResultStatus) -> (Color, &'static str) {
        match status {
            ResultStatus::Ok => (palette::GREEN, "✓"),
            ResultStatus::Warning => (palette::YELLOW, "!"),
            ResultStatus::Error => (palette::RED, "✗"),
            ResultStatus::Info => (palette::DIM, "·"),
        }
    }
}

impl<'a> Widget for ResultsPanel<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(format!(" {} ", self.title))
            .borders(Borders::ALL)
            .border_set(border::ROUNDED)
            .border_style(Style::default().fg(palette::DIM));

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
                        .fg(palette::CYAN)
                        .add_modifier(Modifier::BOLD),
                );
            } else if !result.label.is_empty() || !result.value.is_empty() {
                let (color, sym) = Self::status_style(result.status);
                let line = Line::from(vec![
                    Span::styled(format!(" {} ", sym), Style::default().fg(color)),
                    Span::styled(
                        format!("{:<18}", result.label),
                        Style::default().fg(palette::FG),
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
            .title(" ⌨ Help ")
            .borders(Borders::ALL)
            .border_set(border::ROUNDED)
            .border_style(Style::default().fg(palette::CYAN));

        let inner = block.inner(area);
        block.render(area, buf);

        let logo = [
            ("", palette::CYAN),
            ("  ╭───────────────────────────────╮", palette::CYAN),
            ("  │  ⌨  KEYBOARD TESTKIT  v0.1   │", palette::CYAN),
            ("  ╰───────────────────────────────╯", palette::CYAN),
            ("", palette::DIM),
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
                    .fg(palette::CYAN)
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
                        Style::default().fg(palette::YELLOW),
                    ),
                    Span::styled(*desc, Style::default().fg(palette::DIM)),
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
        let bg = Style::default().bg(palette::BG).fg(palette::FG);
        for x in area.x..area.x + area.width {
            buf.set_string(x, area.y, " ", bg);
        }

        let (icon, color) = if self.state == "RUNNING" {
            ("▶", palette::GREEN)
        } else {
            ("▪", palette::YELLOW)
        };

        buf.set_string(
            area.x + 1,
            area.y,
            icon,
            Style::default().bg(palette::BG).fg(color),
        );

        let left = format!(" {} │ {}", self.state, self.view);
        buf.set_string(area.x + 2, area.y, &left, bg.add_modifier(Modifier::BOLD));

        if let Some(msg) = self.message {
            let x = area.x + (area.width / 2).saturating_sub(msg.len() as u16 / 2);
            buf.set_string(
                x,
                area.y,
                msg,
                Style::default().bg(palette::BG).fg(palette::YELLOW),
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
}

impl<'a> TabBar<'a> {
    pub fn new(tabs: &'a [&'a str], selected: usize) -> Self {
        Self { tabs, selected }
    }
}

impl<'a> Widget for TabBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let bg = Style::default().bg(palette::BG);
        for x in area.x..area.x + area.width {
            buf.set_string(x, area.y, " ", bg);
        }

        let mut x = area.x + 1;
        for (i, tab) in self.tabs.iter().enumerate() {
            let sel = i == self.selected;
            let style = if sel {
                Style::default()
                    .fg(palette::CYAN)
                    .bg(palette::BG)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette::DIM).bg(palette::BG)
            };

            let num_style = if sel {
                Style::default()
                    .fg(palette::YELLOW)
                    .bg(palette::BG)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette::DIM).bg(palette::BG)
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
