//! Keyboard TestKit - Portable keyboard testing utility
//!
//! A single-executable keyboard diagnostic tool for USB portability.

use anyhow::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode as CtKeyCode, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    symbols::border,
    widgets::{Block, Borders},
    Terminal,
};
use std::{io::stdout, sync::mpsc};

use keyboard_testkit::{
    config::Config,
    keyboard::{KeyEvent, KeyboardListener},
    ui::{App, AppState, AppView, HelpPanel, KeyboardVisual, ResultsPanel, StatusBar, TabBar},
};

#[cfg(target_os = "linux")]
use keyboard_testkit::keyboard::{evdev_status, EvdevListener};

fn main() -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create application
    let config = Config::default();
    let mut app = App::new(config.clone());

    // Create keyboard event channel
    let (event_tx, event_rx) = mpsc::channel::<KeyEvent>();

    // Create keyboard listener (device_query based - fallback)
    let mut listener = KeyboardListener::new(event_tx.clone());

    // On Linux, try to use evdev for better OEM key detection
    #[cfg(target_os = "linux")]
    let mut evdev_listener = {
        match EvdevListener::try_new(event_tx) {
            Some(evdev) => {
                app.set_status(format!("Evdev: {}", evdev_status()));
                Some(evdev)
            }
            None => {
                app.set_status(
                    "Evdev unavailable - using fallback (limited OEM key support)".to_string(),
                );
                None
            }
        }
    };

    #[cfg(target_os = "linux")]
    let _use_evdev = evdev_listener.is_some();

    #[cfg(not(target_os = "linux"))]
    let _use_evdev = false;

    // Main loop
    let tick_rate = config.refresh_interval();

    loop {
        // Poll keyboard state - use evdev on Linux if available, otherwise fallback
        #[cfg(target_os = "linux")]
        {
            if let Some(ref mut evdev) = evdev_listener {
                evdev.poll();
            } else {
                listener.poll();
            }
        }

        #[cfg(not(target_os = "linux"))]
        listener.poll();

        // Process keyboard events
        while let Ok(key_event) = event_rx.try_recv() {
            app.process_event(&key_event);
        }

        // Draw UI
        terminal.draw(|frame| {
            let size = frame.area();

            // Create layout
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1), // Tab bar
                    Constraint::Length(7), // Keyboard visual
                    Constraint::Min(10),   // Main content
                    Constraint::Length(1), // Status bar
                ])
                .split(size);

            // Tab bar
            let tab_names: Vec<&str> = AppView::all().iter().map(|v| v.name()).collect();
            let tab_bar = TabBar::new(&tab_names, app.view.index());
            frame.render_widget(tab_bar, chunks[0]);

            // Keyboard visual
            let kb_block = Block::default()
                .title(" ⌨ Keyboard ")
                .borders(Borders::ALL)
                .border_set(border::ROUNDED)
                .border_style(Style::default().fg(Color::Rgb(90, 90, 110)));
            let kb_inner = kb_block.inner(chunks[1]);
            frame.render_widget(kb_block, chunks[1]);
            let kb_visual = KeyboardVisual::new(&app.keyboard_state);
            frame.render_widget(kb_visual, kb_inner);

            // Main content area
            match app.view {
                AppView::Help => {
                    frame.render_widget(HelpPanel, chunks[2]);
                }
                _ => {
                    let results = app.current_results();
                    let panel = ResultsPanel::new(&results, app.view.name());
                    frame.render_widget(panel, chunks[2]);
                }
            }

            // Status bar
            let state_str = match app.state {
                AppState::Running => "RUNNING",
                AppState::Paused => "PAUSED",
                AppState::Quitting => "QUITTING",
            };
            let elapsed = app.elapsed_formatted();
            let status = StatusBar::new(state_str, app.view.name(), &elapsed, app.total_events)
                .message(app.get_status());
            frame.render_widget(status, chunks[3]);
        })?;

        // Handle terminal events (for navigation/control)
        if event::poll(tick_rate)? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    CtKeyCode::Char('q') | CtKeyCode::Esc => {
                        app.quit();
                    }
                    CtKeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => {
                        app.prev_view();
                    }
                    CtKeyCode::Tab => {
                        app.next_view();
                    }
                    CtKeyCode::Char('m') => app.toggle_shortcuts(),
                    CtKeyCode::Char('1') if app.shortcuts_enabled => app.view = AppView::Dashboard,
                    CtKeyCode::Char('2') if app.shortcuts_enabled => {
                        app.view = AppView::PollingRate
                    }
                    CtKeyCode::Char('3') if app.shortcuts_enabled => {
                        app.view = AppView::HoldRelease
                    }
                    CtKeyCode::Char('4') if app.shortcuts_enabled => app.view = AppView::Stickiness,
                    CtKeyCode::Char('5') if app.shortcuts_enabled => app.view = AppView::Rollover,
                    CtKeyCode::Char('6') if app.shortcuts_enabled => app.view = AppView::Latency,
                    CtKeyCode::Char('7') if app.shortcuts_enabled => app.view = AppView::Shortcuts,
                    CtKeyCode::Char('8') if app.shortcuts_enabled => app.view = AppView::Virtual,
                    CtKeyCode::Char('9') if app.shortcuts_enabled => app.view = AppView::OemKeys,
                    CtKeyCode::Char('0') if app.shortcuts_enabled => app.view = AppView::Help,
                    CtKeyCode::Char('v') => {
                        // Trigger virtual key test when on Virtual view
                        if app.view == AppView::Virtual {
                            app.virtual_test.request_virtual_test();
                        }
                    }
                    CtKeyCode::Char('a') => {
                        // Add mapping for last detected unknown key when on OEM view
                        if app.view == AppView::OemKeys {
                            app.add_oem_mapping_for_last_unknown();
                        }
                    }
                    CtKeyCode::Char('f') => {
                        // Cycle FN mode when on OEM view
                        if app.view == AppView::OemKeys {
                            app.cycle_fn_mode();
                        }
                    }
                    CtKeyCode::Char('c') => {
                        // Clear OEM mappings when on OEM view
                        if app.view == AppView::OemKeys {
                            app.clear_oem_mappings();
                        }
                    }
                    CtKeyCode::Char('?') => app.view = AppView::Help,
                    CtKeyCode::Char(' ') => app.toggle_pause(),
                    CtKeyCode::Char('r') => app.reset_current(),
                    CtKeyCode::Char('R') => app.reset_all(),
                    CtKeyCode::Char('e') => {
                        let filename = format!(
                            "keyboard_report_{}.json",
                            chrono::Utc::now().format("%Y%m%d_%H%M%S")
                        );
                        let _ = app.export_report(&filename);
                    }
                    _ => {}
                }
            }
        }

        // Execute pending virtual key sends
        if app.virtual_test.has_pending_send() {
            match app.virtual_test.execute_virtual_send() {
                Ok(()) => app.set_status("Virtual keys sent (z, x, c)".to_string()),
                Err(e) => app.set_status(format!("Virtual send failed: {}", e)),
            }
        }

        // Check if we should quit
        if app.state == AppState::Quitting {
            break;
        }
    }

    // Cleanup terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    println!("\nKeyboard TestKit session complete.");
    println!("Total events processed: {}", app.total_events);
    println!("Session duration: {}", app.elapsed_formatted());

    Ok(())
}
