//! Keyboard TestKit - Portable keyboard testing utility
//!
//! A single-executable keyboard diagnostic tool for USB portability.
//!
//! ## Mapper Daemon Mode (Linux)
//!
//! Run as a key mapping daemon for special keyboard keys:
//! ```bash
//! sudo keyboard-testkit --mapper                    # Use config file mappings
//! sudo keyboard-testkit --mapper --preset asus-g14  # Use ASUS G14 preset
//! sudo keyboard-testkit --mapper-install             # Install as systemd service
//! sudo keyboard-testkit --mapper-uninstall           # Remove systemd service
//! sudo keyboard-testkit --list-presets               # Show available presets
//! ```

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

// Lightweight logging macros to replace log + env_logger crates.
// info!/warn!/error! print to stderr; debug! is no-op in release builds.
macro_rules! info    { ($($arg:tt)*) => { eprintln!("[INFO]  {}", format_args!($($arg)*)) } }
macro_rules! warn    { ($($arg:tt)*) => { eprintln!("[WARN]  {}", format_args!($($arg)*)) } }
macro_rules! error   { ($($arg:tt)*) => { eprintln!("[ERROR] {}", format_args!($($arg)*)) } }
macro_rules! debug   { ($($arg:tt)*) => { if cfg!(debug_assertions) { eprintln!("[DEBUG] {}", format_args!($($arg)*)); } } }

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode as CtKeyCode,
        KeyEvent as CtKeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::Style,
    symbols::border,
    widgets::{Block, Borders},
    Terminal,
};
use std::io::stdout;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use keyboard_testkit::{
    config::Config,
    keyboard::{KeyEvent, KeyboardListener},
    tests::{Finding, InputSource},
    ui::{
        App, AppState, AppView, HelpPanel, KeyboardVisual, ResultsPanel, SettingsPanel,
        ShortcutOverlay, StatusBar, TabBar,
    },
};

#[cfg(target_os = "linux")]
use keyboard_testkit::keyboard::{evdev_status, EvdevListener};

#[cfg(target_os = "linux")]
use keyboard_testkit::mapper;

/// Restore the terminal to its original state.
///
/// Called from the panic hook, signal handler cleanup, and normal exit.
fn restore_terminal() {
    if KEYBOARD_ENHANCED.load(Ordering::SeqCst) {
        let _ = execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
    }
    let _ = disable_raw_mode();
    let _ = execute!(std::io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
}

/// Whether kitty keyboard-protocol enhancement flags were pushed (must be popped on exit).
static KEYBOARD_ENHANCED: AtomicBool = AtomicBool::new(false);

/// Try to enable key-release reporting (kitty keyboard protocol).
///
/// Returns true when the terminal supports it and the flags were pushed.
fn enable_keyboard_enhancement() -> bool {
    match crossterm::terminal::supports_keyboard_enhancement() {
        Ok(true) => {
            let flags = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES;
            if execute!(std::io::stdout(), PushKeyboardEnhancementFlags(flags)).is_ok() {
                KEYBOARD_ENHANCED.store(true, Ordering::SeqCst);
                return true;
            }
            false
        }
        _ => false,
    }
}

/// Global flag for signal handler to indicate shutdown.
static SIGNAL_RUNNING: std::sync::OnceLock<Arc<AtomicBool>> = std::sync::OnceLock::new();

/// Install a signal handler that sets the running flag to false on SIGINT/SIGTERM.
fn install_signal_handler(running: Arc<AtomicBool>) {
    SIGNAL_RUNNING.get_or_init(|| running.clone());

    #[cfg(unix)]
    {
        // SAFETY: The handler only performs an atomic store, which is
        // async-signal-safe. SIGNAL_RUNNING is initialized above before
        // signal() is called.
        unsafe {
            extern "C" fn handler(_sig: libc::c_int) {
                if let Some(flag) = SIGNAL_RUNNING.get() {
                    flag.store(false, Ordering::SeqCst);
                }
            }
            libc::signal(libc::SIGINT, handler as *const () as libc::sighandler_t);
            libc::signal(libc::SIGTERM, handler as *const () as libc::sighandler_t);
        }
    }

    // On non-Unix platforms, Ctrl+C is handled by crossterm's raw mode event loop.
    #[cfg(not(unix))]
    {
        let _ = running;
    }
}

/// Parse CLI arguments and return the mode to run
fn parse_args() -> CliMode {
    let args: Vec<String> = std::env::args().collect();

    // Check for mapper-related flags
    if args.iter().any(|a| a == "--mapper") {
        let preset = args
            .windows(2)
            .find(|w| w[0] == "--preset")
            .map(|w| w[1].clone());

        let device = args
            .windows(2)
            .find(|w| w[0] == "--device")
            .map(|w| std::path::PathBuf::from(&w[1]));

        return CliMode::Mapper { preset, device };
    }

    if args.iter().any(|a| a == "--mapper-install") {
        let preset = args
            .windows(2)
            .find(|w| w[0] == "--preset")
            .map(|w| w[1].clone());
        return CliMode::MapperInstall { preset };
    }

    if args.iter().any(|a| a == "--mapper-uninstall") {
        return CliMode::MapperUninstall;
    }

    if args.iter().any(|a| a == "--list-presets") {
        return CliMode::ListPresets;
    }

    if args.iter().any(|a| a == "--list-devices") {
        return CliMode::ListDevices;
    }

    if args.iter().any(|a| a == "--help" || a == "-h") {
        return CliMode::Help;
    }

    if args.iter().any(|a| a == "--version" || a == "-V") {
        return CliMode::Version;
    }

    CliMode::Tui {
        auto_start: args.iter().any(|a| a == "--auto" || a == "--auto-test"),
    }
}

/// CLI operating mode
enum CliMode {
    /// Normal TUI mode; `auto_start` launches straight into the auto test
    Tui { auto_start: bool },
    /// Run as key mapper daemon
    Mapper {
        preset: Option<String>,
        device: Option<std::path::PathBuf>,
    },
    /// Install mapper as systemd service
    MapperInstall { preset: Option<String> },
    /// Uninstall mapper systemd service
    MapperUninstall,
    /// List available presets
    ListPresets,
    /// List input devices
    ListDevices,
    /// Show help
    Help,
    /// Show version
    Version,
}

fn main() -> Result<()> {
    let mode = parse_args();

    let auto_start = match mode {
        CliMode::Help => {
            print_help();
            return Ok(());
        }

        CliMode::Version => {
            println!("keyboard-testkit {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }

        #[cfg(target_os = "linux")]
        CliMode::ListPresets => {
            println!("Available key mapping presets:\n");
            for (name, desc) in mapper::MapperPreset::available() {
                println!("  {:15} {}", name, desc);
            }
            println!("\nUsage: keyboard-testkit --mapper --preset <name>");
            return Ok(());
        }

        #[cfg(target_os = "linux")]
        CliMode::ListDevices => {
            println!("Detected input devices:\n");
            match mapper::find_mapper_devices(None) {
                Ok(devices) => {
                    for (path, name) in &devices {
                        println!("  {} - {}", path.display(), name);
                    }
                    println!("\nUsage: keyboard-testkit --mapper --device <path>");
                }
                Err(e) => {
                    println!("Error: {}", e);
                    println!("Try running with sudo for device access.");
                }
            }
            return Ok(());
        }

        #[cfg(target_os = "linux")]
        CliMode::Mapper { preset, device } => {
            info!(
                "Keyboard TestKit v{} — Mapper Daemon",
                env!("CARGO_PKG_VERSION")
            );

            let running = Arc::new(AtomicBool::new(true));
            install_signal_handler(running.clone());

            if let Err(e) = mapper::run_mapper(preset.as_deref(), device, &[], running) {
                error!("Mapper error: {}", e);
                return Err(e.into());
            }
            return Ok(());
        }

        #[cfg(target_os = "linux")]
        CliMode::MapperInstall { preset } => {
            info!("Installing key mapper service...");
            if let Err(e) = mapper::install_service(preset.as_deref()) {
                error!("Install error: {}", e);
                return Err(e.into());
            }
            return Ok(());
        }

        #[cfg(target_os = "linux")]
        CliMode::MapperUninstall => {
            info!("Uninstalling key mapper service...");
            if let Err(e) = mapper::uninstall_service() {
                error!("Uninstall error: {}", e);
                return Err(e.into());
            }
            return Ok(());
        }

        #[cfg(not(target_os = "linux"))]
        CliMode::Mapper { .. }
        | CliMode::MapperInstall { .. }
        | CliMode::MapperUninstall
        | CliMode::ListPresets
        | CliMode::ListDevices => {
            eprintln!("Key mapper daemon is only supported on Linux.");
            return Ok(());
        }

        CliMode::Tui { auto_start } => auto_start,
    };

    // Normal TUI mode
    info!("Keyboard TestKit v{}", env!("CARGO_PKG_VERSION"));

    // Install panic hook so a panic always restores the terminal first
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        original_hook(info);
    }));

    // Setup terminal
    enable_raw_mode()?;
    // Ask for key-release events where the terminal supports them
    // (kitty, WezTerm, foot, Ghostty, Alacritty 0.13+). Windows consoles
    // report releases natively.
    let release_reporting = cfg!(windows) || enable_keyboard_enhancement();
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Ctrl+C is handled via crossterm key events in raw mode.
    // The `running` flag is set to false when the user presses q/Esc.
    let running = Arc::new(AtomicBool::new(true));
    install_signal_handler(running.clone());

    // Run the application; cleanup runs regardless of success or failure
    let result = run_app(&mut terminal, running, release_reporting, auto_start);

    // Cleanup terminal — always runs, even after signal or error
    if KEYBOARD_ENHANCED.swap(false, Ordering::SeqCst) {
        let _ = execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags);
    }
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Ok(Some(summary)) = &result {
        println!("\nKeyboard TestKit session complete.");
        println!("Total events processed: {}", summary.total_events);
        println!("Session duration: {}", summary.elapsed);
        let issues: Vec<&Finding> = summary.findings.iter().filter(|f| f.is_issue()).collect();
        println!("Issues found: {}", issues.len());
        for f in issues {
            println!("  [{}] {}: {}", f.severity.label(), f.category, f.title);
            if !f.detail.is_empty() {
                println!("      {}", f.detail);
            }
        }
    }

    result.map(|_| ())
}

/// What the TUI session reports on exit.
struct SessionOutcome {
    total_events: u64,
    elapsed: String,
    findings: Vec<Finding>,
}

fn print_help() {
    println!("Keyboard TestKit v{}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("USAGE:");
    println!("  keyboard-testkit              Launch the interactive TUI");
    println!("  keyboard-testkit --auto       Launch straight into the automatic diagnostic");
    println!("  keyboard-testkit --mapper     Run as key mapping daemon (Linux, needs root)");
    println!();
    println!("OPTIONS:");
    println!("  -h, --help                    Show this help message");
    println!("  -V, --version                 Show the version");
    println!("  --auto                        Start the guided auto test immediately");
    println!();
    println!("MAPPER OPTIONS (Linux only):");
    println!("  --mapper                      Run as a key mapping daemon");
    println!("  --preset <name>               Use a vendor preset (e.g. asus-g14)");
    println!("  --device <path>               Target specific input device");
    println!("  --mapper-install              Install as a systemd service (runs on boot)");
    println!("  --mapper-uninstall            Remove the systemd service");
    println!("  --list-presets                List available vendor presets");
    println!("  --list-devices                List detected input devices");
    println!();
    println!("EXAMPLES:");
    println!("  # Map ASUS G14 special keys");
    println!("  sudo keyboard-testkit --mapper --preset asus-g14");
    println!();
    println!("  # Install as startup service with ASUS G14 preset");
    println!("  sudo keyboard-testkit --mapper-install --preset asus-g14");
    println!();
    println!("  # Use a specific device");
    println!("  sudo keyboard-testkit --mapper --device /dev/input/event5");
    println!();
    println!("  # Custom mappings via config file");
    println!("  # Edit ~/.config/keyboard-testkit/config.toml:");
    println!("  # [oem_keys]");
    println!("  # key_mappings = [[148, 125], [149, 228]]");
    println!();
    println!("TUI CONTROLS:");
    println!("  Tab/Shift+Tab    Navigate views");
    println!("  1-9,0            Jump to view (when shortcuts enabled)");
    println!("  A                Run the automatic diagnostic (Ctrl+N skip step, Ctrl+C abort)");
    println!("  Space            Pause/Resume");
    println!("  q/Esc/Ctrl+C     Quit");
    println!("  ?                Help");
    println!();
    println!("INPUT:");
    println!("  On Linux, raw evdev access (sudo, or membership of the 'input' group) gives");
    println!("  precise timestamps and real key releases. Otherwise the terminal is used;");
    println!("  terminals with the kitty keyboard protocol still report key releases.");
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    running: Arc<AtomicBool>,
    release_reporting: bool,
    auto_start: bool,
) -> Result<Option<SessionOutcome>> {
    // TODO: Support --config <path> CLI argument for custom config locations.
    let config = Config::load().unwrap_or_else(|e| {
        warn!("Failed to load config: {}. Using defaults.", e);
        Config::default()
    });
    debug!(
        "Config: polling={}s/{}ms, refresh={}Hz, theme={:?}",
        config.polling.test_duration_secs,
        config.polling.sample_window_ms,
        config.ui.refresh_rate_hz,
        config.ui.theme,
    );
    let mut app = App::new(config.clone());

    // Create keyboard event channel
    let (event_tx, event_rx) = mpsc::channel::<KeyEvent>();

    // Create keyboard listener (crossterm-based fallback for non-Linux)
    let mut listener = KeyboardListener::new(event_tx.clone());
    listener.set_release_reporting(release_reporting);

    // On Linux, try to use evdev for better OEM key detection
    #[cfg(target_os = "linux")]
    let mut evdev_listener = {
        match EvdevListener::try_new(event_tx) {
            Some(evdev) => {
                let status = evdev_status();
                app.set_status(format!("Evdev: {}", status));
                Some(evdev)
            }
            None => {
                app.set_status(
                    "Evdev unavailable - using terminal input (run with sudo for raw access)"
                        .to_string(),
                );
                None
            }
        }
    };

    #[cfg(target_os = "linux")]
    let use_evdev = evdev_listener.is_some();

    #[cfg(not(target_os = "linux"))]
    let use_evdev = false;

    let input_source = if use_evdev {
        InputSource::Evdev
    } else if release_reporting {
        InputSource::TerminalWithRelease
    } else {
        InputSource::TerminalPressOnly
    };
    app.set_input_source(input_source);
    info!("Input source: {}", input_source.name());

    if auto_start {
        app.start_auto_test();
    }

    // Main loop
    let tick_rate = config.refresh_interval();

    loop {
        // Check if Ctrl+C / SIGTERM was received
        if !running.load(Ordering::SeqCst) {
            break;
        }

        // Poll keyboard state - use evdev on Linux if available, otherwise
        // let the terminal listener synthesize any overdue key releases.
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

        // Timers: auto-test steps, stuck keys, live findings
        app.tick();

        // Get current theme
        let colors = app.theme_colors;

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
            let tab_bar = TabBar::new(&tab_names, app.view.index()).theme(colors);
            frame.render_widget(tab_bar, chunks[0]);

            // Keyboard visual
            let kb_block = Block::default()
                .title(" ⌨ Keyboard ")
                .borders(Borders::ALL)
                .border_set(border::ROUNDED)
                .border_style(Style::default().fg(colors.dim));
            let kb_inner = kb_block.inner(chunks[1]);
            frame.render_widget(kb_block, chunks[1]);
            let mut kb_visual = KeyboardVisual::new(&app.keyboard_state)
                .theme(colors)
                .layout(app.keyboard_layout);
            if app.auto_test.is_running() || app.auto_test.is_complete() {
                kb_visual = kb_visual.verified(app.auto_test.verified_keys());
            }
            frame.render_widget(kb_visual, kb_inner);

            // Main content area
            match app.view {
                AppView::Help => {
                    frame.render_widget(HelpPanel::new().theme(colors), chunks[2]);
                }
                AppView::Settings => {
                    let items = app.settings_items();
                    let panel = SettingsPanel::new(&items, app.settings_selected).theme(colors);
                    frame.render_widget(panel, chunks[2]);
                }
                _ => {
                    let results = app.current_results();
                    let panel = ResultsPanel::new(&results, app.view.name()).theme(colors);
                    frame.render_widget(panel, chunks[2]);
                }
            }

            // Shortcut overlay (shown in any view)
            if let Some((combo, desc)) = app.shortcut_overlay() {
                let overlay = ShortcutOverlay::new(combo).description(desc).theme(colors);
                frame.render_widget(overlay, chunks[2]);
            }

            // Status bar
            let state_str = match app.state {
                AppState::Running => {
                    if app.auto_test.is_running() {
                        "AUTO TEST"
                    } else {
                        "RUNNING"
                    }
                }
                AppState::Paused => "PAUSED",
                AppState::Quitting => "QUITTING",
            };
            let elapsed = app.elapsed_formatted();
            let status = StatusBar::new(state_str, app.view.name(), &elapsed, app.total_events)
                .message(app.get_status())
                .theme(colors);
            frame.render_widget(status, chunks[3]);
        })?;

        // Handle terminal events (for navigation/control). Drain everything
        // that is queued so a burst of keys is not spread over several frames.
        if event::poll(tick_rate)? {
            loop {
                if let Event::Key(key) = event::read()? {
                    handle_terminal_key(&mut app, &mut listener, use_evdev, key);
                }
                if app.state == AppState::Quitting || !event::poll(Duration::ZERO)? {
                    break;
                }
            }
        }

        // Execute pending virtual key sends
        if app.virtual_test.has_pending_send() {
            match app.virtual_test.execute_virtual_send() {
                Ok(()) => {
                    debug!("Virtual keys sent (z, x, c)");
                    app.set_status("Virtual keys sent (z, x, c)".to_string());
                }
                Err(e) => {
                    error!("Virtual send failed: {}", e);
                    app.set_status(format!("Virtual send failed: {}", e));
                }
            }
        }

        // Check if we should quit
        if app.state == AppState::Quitting {
            break;
        }
    }

    app.refresh_findings();
    info!(
        "Session ended: {} events in {}",
        app.total_events,
        app.elapsed_formatted()
    );
    Ok(Some(SessionOutcome {
        total_events: app.total_events,
        elapsed: app.elapsed_formatted(),
        findings: app.findings.clone(),
    }))
}

/// Feed a terminal key event to the test pipeline and handle UI controls.
fn handle_terminal_key(
    app: &mut App,
    listener: &mut KeyboardListener,
    use_evdev: bool,
    key: CtKeyEvent,
) {
    // The keystroke that starts the auto test must not be counted by the
    // test it starts (it would show up as phantom input in the idle step).
    let starts_auto_test = key.kind == KeyEventKind::Press
        && key.code == CtKeyCode::Char('A')
        && !app.auto_test_running()
        && app.view != AppView::Settings;

    // When evdev is not in use, the terminal is the test input. Presses,
    // repeats and releases each mean something different to the tests.
    if !use_evdev && !starts_auto_test {
        match key.kind {
            KeyEventKind::Press => {
                listener.send_press(key.code);
            }
            KeyEventKind::Repeat => listener.send_repeat(key.code),
            KeyEventKind::Release => {
                listener.send_release(key.code);
            }
        }
    }

    // Controls only fire on the initial press, never on repeat or release.
    if key.kind != KeyEventKind::Press {
        return;
    }

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    // Ctrl+C: abort a running auto test, otherwise quit.
    if ctrl && matches!(key.code, CtKeyCode::Char('c') | CtKeyCode::Char('C')) {
        if app.auto_test_running() {
            app.abort_auto_test();
        } else {
            app.quit();
        }
        return;
    }

    // While the auto test runs, every key is test input: only the two
    // Ctrl chords above/below are interpreted, so 'q', 'r', Esc... can be
    // verified like any other key.
    if app.auto_test_running() {
        if ctrl && matches!(key.code, CtKeyCode::Char('n') | CtKeyCode::Char('N')) {
            app.skip_auto_step();
        }
        return;
    }

    // Settings view has its own key handling
    if app.view == AppView::Settings {
        match key.code {
            CtKeyCode::Char('q') | CtKeyCode::Esc => {
                app.view = AppView::Dashboard; // Back to dashboard
            }
            CtKeyCode::Up => {
                if app.settings_selected > 0 {
                    app.settings_selected -= 1;
                }
            }
            CtKeyCode::Down => {
                let item_count = app.settings_items().len();
                if app.settings_selected + 1 < item_count {
                    app.settings_selected += 1;
                }
            }
            CtKeyCode::Right => {
                app.adjust_setting(true);
            }
            CtKeyCode::Left => {
                app.adjust_setting(false);
            }
            CtKeyCode::Char('s') => {
                app.save_config();
            }
            CtKeyCode::Tab => {
                app.next_view();
            }
            _ => {}
        }
        return;
    }

    match key.code {
        CtKeyCode::Char('q') | CtKeyCode::Esc => {
            app.quit();
        }
        CtKeyCode::BackTab => {
            app.prev_view();
        }
        CtKeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => {
            app.prev_view();
        }
        CtKeyCode::Tab => {
            app.next_view();
        }
        CtKeyCode::Char('m') => app.toggle_shortcuts(),
        CtKeyCode::Char('t') => app.toggle_theme(),
        CtKeyCode::Char('S') => {
            app.view = AppView::Settings;
        }
        CtKeyCode::Char('A') => app.start_auto_test(),
        CtKeyCode::Char('1') if app.shortcuts_enabled => app.view = AppView::Dashboard,
        CtKeyCode::Char('2') if app.shortcuts_enabled => app.view = AppView::PollingRate,
        CtKeyCode::Char('3') if app.shortcuts_enabled => app.view = AppView::HoldRelease,
        CtKeyCode::Char('4') if app.shortcuts_enabled => app.view = AppView::Stickiness,
        CtKeyCode::Char('5') if app.shortcuts_enabled => app.view = AppView::Rollover,
        CtKeyCode::Char('6') if app.shortcuts_enabled => app.view = AppView::Latency,
        CtKeyCode::Char('7') if app.shortcuts_enabled => app.view = AppView::Shortcuts,
        CtKeyCode::Char('8') if app.shortcuts_enabled => app.view = AppView::Virtual,
        CtKeyCode::Char('9') if app.shortcuts_enabled => app.view = AppView::OemKeys,
        CtKeyCode::Char('0') if app.shortcuts_enabled => app.view = AppView::AutoTest,
        CtKeyCode::Char('v') => {
            if app.view == AppView::Virtual {
                app.virtual_test.request_virtual_test();
            }
        }
        CtKeyCode::Char('a') => {
            if app.view == AppView::OemKeys {
                app.add_oem_mapping_for_last_unknown();
            }
        }
        CtKeyCode::Char('f') => {
            if app.view == AppView::OemKeys {
                app.cycle_fn_mode();
            }
        }
        CtKeyCode::Char('c') => {
            if app.view == AppView::OemKeys {
                app.clear_oem_mappings();
            }
        }
        CtKeyCode::Char('?') => app.view = AppView::Help,
        CtKeyCode::Char(' ') => app.toggle_pause(),
        CtKeyCode::Char('r') => app.reset_current(),
        CtKeyCode::Char('R') => app.reset_all(),
        CtKeyCode::Char('e') => {
            let filename = {
                use std::time::SystemTime;
                let secs = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                format!("keyboard_report_{}.json", secs)
            };
            match app.export_report(&filename) {
                Ok(_) => info!("Report exported to {}", filename),
                Err(e) => {
                    error!("Export failed: {}", e);
                    app.set_status(format!("Export failed: {}", e));
                }
            }
        }
        _ => {}
    }
}
