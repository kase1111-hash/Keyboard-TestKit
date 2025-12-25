//! Custom TUI widgets

use crate::tests::{TestResult, ResultStatus};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

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
            ResultStatus::Ok => Color::Green,
            ResultStatus::Warning => Color::Yellow,
            ResultStatus::Error => Color::Red,
            ResultStatus::Info => Color::Cyan,
        }
    }

    fn status_symbol(status: ResultStatus) -> &'static str {
        match status {
            ResultStatus::Ok => "[OK]",
            ResultStatus::Warning => "[!!]",
            ResultStatus::Error => "[XX]",
            ResultStatus::Info => "[--]",
        }
    }
}

impl<'a> Widget for ResultsPanel<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title(self.title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::White));

        let inner = block.inner(area);
        block.render(area, buf);

        let mut y = inner.y;
        for result in self.results {
            if y >= inner.y + inner.height {
                break;
            }

            let color = Self::status_color(result.status);
            let symbol = Self::status_symbol(result.status);

            let line = Line::from(vec![
                Span::styled(
                    format!("{} ", symbol),
                    Style::default().fg(color),
                ),
                Span::styled(
                    format!("{}: ", result.label),
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    &result.value,
                    Style::default().fg(color),
                ),
            ]);

            buf.set_line(inner.x, y, &line, inner.width);
            y += 1;
        }
    }
}

/// Widget for the help screen
pub struct HelpPanel;

impl Widget for HelpPanel {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .title("Help - Keyboard TestKit")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let inner = block.inner(area);
        block.render(area, buf);

        let help_text = vec![
            "",
            " NAVIGATION",
            " -----------",
            " Tab / Shift+Tab  : Switch between test views",
            " 1-8              : Jump to specific view",
            " q / Esc          : Quit application",
            "",
            " CONTROLS",
            " -----------",
            " Space            : Pause/Resume testing",
            " r                : Reset current test",
            " R                : Reset all tests",
            " e                : Export report to JSON",
            " v                : Send virtual test keys (on Virtual view)",
            " ?                : Show this help",
            "",
            " TESTS",
            " -----------",
            " 1. Dashboard     : Overview of all tests",
            " 2. Polling       : Measure keyboard polling rate & jitter",
            " 3. Hold/Bounce   : Key bounce detection & hold timing",
            " 4. Sticky        : Detect stuck/sticky keys",
            " 5. NKRO          : Test N-key rollover & ghosting",
            " 6. Latency       : Measure per-key input latency",
            " 7. Shortcuts     : Detect shortcuts & conflicts",
            " 8. Virtual       : Detect virtual/automated input",
            "",
            " Press any key to start testing!",
        ];

        for (i, line) in help_text.iter().enumerate() {
            if i as u16 >= inner.height {
                break;
            }
            let style = if line.starts_with(" ") && line.contains("---") {
                Style::default().fg(Color::DarkGray)
            } else if line.starts_with(" ") && line.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            buf.set_string(inner.x, inner.y + i as u16, line, style);
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
        let bg_style = Style::default().bg(Color::DarkGray).fg(Color::White);
        for x in area.x..area.x + area.width {
            buf.set_string(x, area.y, " ", bg_style);
        }

        // Left side: state and view
        let left = format!(" {} | {} ", self.state, self.view);
        buf.set_string(area.x, area.y, &left, bg_style.add_modifier(Modifier::BOLD));

        // Center: message if any
        if let Some(msg) = self.message {
            let msg_style = Style::default().bg(Color::DarkGray).fg(Color::Yellow);
            let msg_x = area.x + (area.width / 2).saturating_sub(msg.len() as u16 / 2);
            buf.set_string(msg_x, area.y, msg, msg_style);
        }

        // Right side: elapsed time and events
        let right = format!(" {} | Events: {} ", self.elapsed, self.events);
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
        let mut x = area.x;

        for (i, tab) in self.tabs.iter().enumerate() {
            let is_selected = i == self.selected;

            let style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::DarkGray)
            };

            let label = format!(" {} ", tab);
            let width = label.len() as u16;

            if x + width <= area.x + area.width {
                buf.set_string(x, area.y, &label, style);
                x += width;

                // Separator
                if i < self.tabs.len() - 1 && x < area.x + area.width {
                    buf.set_string(x, area.y, "|", Style::default().fg(Color::DarkGray));
                    x += 1;
                }
            }
        }

        // Fill rest with background
        for fill_x in x..area.x + area.width {
            buf.set_string(fill_x, area.y, " ", Style::default().bg(Color::DarkGray));
        }
    }
}
