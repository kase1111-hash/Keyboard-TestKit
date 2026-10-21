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
    tests::{Finding, InputSource, Severity},
    ui::{
        App, AppState, AppView, HelpPanel, KeyboardVisual, ResultsPanel, SettingsPanel,
        ShortcutOverlay, StatusBar, TabBar,
    },
};

#[cfg(target_os = "linux")]
use keyboard_testkit::keyboard::{evdev_status, EvdevDeviceChange, EvdevListener};

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
        let mut findings = summary.findings.clone();
        if let Some(finding) = disconnect_finding(summary.keyboard_disconnects) {
            findings.insert(0, finding);
        }
        let issues: Vec<&Finding> = findings.iter().filter(|f| f.is_issue()).collect();
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
    /// Times an evdev keyboard disappeared during the session
    keyboard_disconnects: u32,
}

/// A keyboard that drops off the bus mid-session (flaky cable or port,
/// weak wireless link) is a fault in its own right, and it explains keys
/// that "never registered" in the meantime.
fn disconnect_finding(disconnects: u32) -> Option<Finding> {
    (disconnects > 0).then(|| Finding {
        severity: Severity::Warning,
        category: "Connection",
        title: format!(
            "Keyboard disconnected {} time(s) during the session",
            disconnects
        ),
        detail: "Keys pressed while it was gone were not seen by any test. A wireless \
                 keyboard that went to sleep while idle shows up here too."
            .to_string(),
        advice: "Check the cable and USB port (try another port), or the wireless \
                 link and batteries."
            .to_string(),
    })
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
    println!("  Esc/Ctrl+C       Quit");
    println!("  Ctrl+P/R/E       Pause, reset current test, export report (any view");
    println!("                   but Shortcuts, where Ctrl combos are test input)");
    println!("  In the test views (Polling ... OEM/FN) every other key is test input.");
    println!("  On the Dashboard, Auto and Help screens these single keys also work:");
    println!("  A                Run the automatic diagnostic (Ctrl+N skip step, Ctrl+C abort)");
    println!("  1-9,0            Jump to view (when shortcuts enabled)");
    println!("  Space            Pause/Resume");
    println!("  r/R  e  q        Reset current/all, export, quit");
    println!("  S  t  m  ?       Settings, theme, number-key shortcuts, help");
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

    // Times an evdev keyboard disappeared (shown in the exit summary)
    #[cfg(target_os = "linux")]
    let mut disconnects = DisconnectLog::default();

    // Main loop
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
                report_device_changes(&mut app, evdev, &mut disconnects);
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
        // The refresh rate is read every frame so Settings changes apply.
        if event::poll(frame_interval(app.config.ui.refresh_rate_hz))? {
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
    #[cfg(target_os = "linux")]
    let keyboard_disconnects = disconnects.count;
    #[cfg(not(target_os = "linux"))]
    let keyboard_disconnects = 0;
    Ok(Some(SessionOutcome {
        total_events: app.total_events,
        elapsed: app.elapsed_formatted(),
        findings: app.findings.clone(),
        keyboard_disconnects,
    }))
}

/// Status shown while evdev has no keyboard to read.
#[cfg(target_os = "linux")]
const NO_KEYBOARD_STATUS: &str =
    "No keyboard connected - plug it in or reconnect it, it is picked up automatically";

/// Keyboard disconnects seen during the session.
///
/// One keyboard usually has several event nodes (keys, media keys, an NKRO
/// interface) that all vanish when it is unplugged, so node disconnects
/// close together are one disconnect.
#[cfg(target_os = "linux")]
#[derive(Debug, Default)]
struct DisconnectLog {
    /// Separate disconnects so far
    count: u32,
    /// When the last node disappeared (cleared when a keyboard comes back)
    last: Option<std::time::Instant>,
}

#[cfg(target_os = "linux")]
impl DisconnectLog {
    /// Node disconnects this close together belong to the same unplug.
    const MERGE_WINDOW: Duration = Duration::from_secs(2);

    /// Record that a node disappeared at `now`.
    fn node_lost(&mut self, now: std::time::Instant) {
        if self
            .last
            .is_none_or(|last| now.saturating_duration_since(last) >= Self::MERGE_WINDOW)
        {
            self.count += 1;
        }
        self.last = Some(now);
    }

    /// A keyboard came back: the next node lost is a new disconnect.
    fn node_found(&mut self) {
        self.last = None;
    }
}

/// Make keyboards that disappear or come back visible in the status line.
///
/// A disconnect also stops a running auto test: its steps would otherwise
/// time out and blame the missing input on dead keys.
#[cfg(target_os = "linux")]
fn report_device_changes(
    app: &mut App,
    evdev: &mut EvdevListener,
    disconnects: &mut DisconnectLog,
) {
    let mut stopped = "";
    for change in evdev.take_device_changes() {
        match change {
            EvdevDeviceChange::Disconnected { name, .. } => {
                disconnects.node_lost(std::time::Instant::now());
                if app.auto_test_running() {
                    app.abort_auto_test();
                    stopped = " - auto test stopped";
                }
                let left = match evdev.device_count() {
                    0 => "no keyboard left".to_string(),
                    n => format!("{} device(s) left", n),
                };
                app.set_status(format!(
                    "Keyboard disconnected: {} ({}){}",
                    name, left, stopped
                ));
            }
            EvdevDeviceChange::Connected { name, .. } => {
                disconnects.node_found();
                app.set_status(format!("Keyboard connected: {}", name));
            }
        }
    }
    // Keep saying so while nothing is connected (other messages expire)
    if evdev.device_count() == 0 && app.get_status().is_none() {
        app.set_status(NO_KEYBOARD_STATUS.to_string());
    }
}

/// Status hint shown when a key-test view is opened.
const TEST_VIEW_HINT: &str =
    "Keys here are test input - Ctrl+P pause, Ctrl+R reset, Ctrl+E export, Esc quit";

/// Status hint shown when the Shortcuts view is opened.
const SHORTCUTS_VIEW_HINT: &str =
    "Keys and Ctrl combos here are test input - Tab to leave, Esc or Ctrl+C quit";

/// What a terminal keystroke asks the app to do, besides being test input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    None,
    Quit,
    AbortAutoTest,
    SkipAutoStep,
    NextView,
    PrevView,
    ShowView(AppView),
    StartAutoTest,
    TogglePause,
    ResetCurrent,
    ResetAll,
    Export,
    ToggleShortcuts,
    ToggleTheme,
    SendVirtualKeys,
    AddOemMapping,
    CycleFnMode,
    ClearOemMappings,
    SettingsBack,
    SettingsUp,
    SettingsDown,
    SettingsIncrease,
    SettingsDecrease,
    SettingsSave,
}

/// Views whose purpose is pressing keys: there every plain key (letters,
/// digits, Space, punctuation, with or without Shift) is test input, so
/// mashing keys can never quit, reset, export or pause by accident.
fn is_key_test_view(view: AppView) -> bool {
    matches!(
        view,
        AppView::PollingRate
            | AppView::HoldRelease
            | AppView::Stickiness
            | AppView::Rollover
            | AppView::Latency
            | AppView::Shortcuts
            | AppView::Virtual
            | AppView::OemKeys
    )
}

/// The character a key stands for as a command key.
///
/// Terminals using the kitty keyboard protocol report Shift+a as 'a' with
/// SHIFT (crossterm only folds the case when comparing whole events), and
/// Shift+/ as '/' with SHIFT; legacy terminals and Windows send 'A' and '?'.
fn command_char(key: &CtKeyEvent) -> Option<char> {
    let CtKeyCode::Char(c) = key.code else {
        return None;
    };
    if key.modifiers.contains(KeyModifiers::SHIFT) {
        if c.is_ascii_lowercase() {
            return Some(c.to_ascii_uppercase());
        }
        if c == '/' {
            return Some('?');
        }
    }
    Some(c)
}

/// Map a terminal keystroke to a command.
///
/// The rule, also shown in the help panel and README:
/// - while the auto test runs, every key is test input except Ctrl+N (skip
///   step) and Ctrl+C (abort);
/// - everywhere else Tab / Shift+Tab switch views, Esc and Ctrl+C quit, and
///   Ctrl+P / Ctrl+R / Ctrl+E pause, reset the current test and export,
///   except in the Shortcuts view, where Ctrl combos are the test input;
/// - single-key commands (q, Space, r, R, e, A, S, t, m, ?, 1-9/0) only work
///   on the Dashboard, Auto and Help screens; in the key-test views every
///   other key is test input, apart from the view's own action keys (v on
///   Virtual; a, f, c on OEM/FN).
fn command_for_key(
    key: &CtKeyEvent,
    view: AppView,
    auto_test_running: bool,
    shortcuts_enabled: bool,
) -> Command {
    // Controls only fire on the initial press, never on repeat or release.
    if key.kind != KeyEventKind::Press {
        return Command::None;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let ch = command_char(key);
    let ctrl_char = if ctrl {
        ch.map(|c| c.to_ascii_lowercase())
    } else {
        None
    };

    // Ctrl+C: abort a running auto test, otherwise quit.
    if ctrl_char == Some('c') {
        return if auto_test_running {
            Command::AbortAutoTest
        } else {
            Command::Quit
        };
    }

    // While the auto test runs, every key is test input: only the two
    // Ctrl chords are interpreted, so 'q', 'r', Esc... can be verified like
    // any other key.
    if auto_test_running {
        return if ctrl_char == Some('n') {
            Command::SkipAutoStep
        } else {
            Command::None
        };
    }

    // Ctrl chords work in every view but Shortcuts, which records Ctrl
    // combos (Ctrl+R, Ctrl+P...) as its test input. Ctrl+Alt is AltGr on
    // Windows, which types characters on many layouts, so it is never a
    // command.
    if ctrl {
        if key.modifiers.contains(KeyModifiers::ALT) || view == AppView::Shortcuts {
            return Command::None;
        }
        return match ctrl_char {
            Some('p') => Command::TogglePause,
            Some('r') => Command::ResetCurrent,
            Some('e') => Command::Export,
            _ => Command::None,
        };
    }

    // Settings view has its own key handling
    if view == AppView::Settings {
        return match key.code {
            CtKeyCode::Char('q') | CtKeyCode::Esc => Command::SettingsBack,
            CtKeyCode::Up => Command::SettingsUp,
            CtKeyCode::Down => Command::SettingsDown,
            CtKeyCode::Right => Command::SettingsIncrease,
            CtKeyCode::Left => Command::SettingsDecrease,
            CtKeyCode::Char('s') => Command::SettingsSave,
            CtKeyCode::Tab => Command::NextView,
            CtKeyCode::BackTab => Command::PrevView,
            _ => Command::None,
        };
    }

    match key.code {
        CtKeyCode::Esc => return Command::Quit,
        CtKeyCode::BackTab => return Command::PrevView,
        CtKeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => return Command::PrevView,
        CtKeyCode::Tab => return Command::NextView,
        _ => {}
    }

    let Some(c) = ch else {
        return Command::None;
    };

    if is_key_test_view(view) {
        return match (view, c) {
            (AppView::Virtual, 'v') => Command::SendVirtualKeys,
            (AppView::OemKeys, 'a') => Command::AddOemMapping,
            (AppView::OemKeys, 'f') => Command::CycleFnMode,
            (AppView::OemKeys, 'c') => Command::ClearOemMappings,
            _ => Command::None,
        };
    }

    // Dashboard, Auto and Help: single-key commands
    match c {
        'q' => Command::Quit,
        ' ' => Command::TogglePause,
        'r' => Command::ResetCurrent,
        'R' => Command::ResetAll,
        'e' => Command::Export,
        'A' => Command::StartAutoTest,
        'S' => Command::ShowView(AppView::Settings),
        '?' => Command::ShowView(AppView::Help),
        'm' => Command::ToggleShortcuts,
        't' => Command::ToggleTheme,
        '1'..='9' | '0' if shortcuts_enabled => {
            // 1-9 are the first nine tabs, 0 the tenth (Auto)
            let index = c.to_digit(10).map_or(0, |d| (d as usize + 9) % 10);
            Command::ShowView(AppView::from_index(index))
        }
        _ => Command::None,
    }
}

/// Feed a terminal key event to the test pipeline and handle UI controls.
fn handle_terminal_key(
    app: &mut App,
    listener: &mut KeyboardListener,
    use_evdev: bool,
    key: CtKeyEvent,
) {
    let command = command_for_key(
        &key,
        app.view,
        app.auto_test_running(),
        app.shortcuts_enabled,
    );

    // When evdev is not in use, the terminal is the test input. Presses,
    // repeats and releases each mean something different to the tests.
    // The keystroke that starts the auto test must not be counted by the
    // test it starts (it would show up as phantom input in the idle step).
    if !use_evdev && command != Command::StartAutoTest {
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

    apply_command(app, command);
}

/// Carry out a command.
fn apply_command(app: &mut App, command: Command) {
    let view_before = app.view;
    match command {
        Command::None => {}
        Command::Quit => app.quit(),
        Command::AbortAutoTest => app.abort_auto_test(),
        Command::SkipAutoStep => app.skip_auto_step(),
        Command::NextView => app.next_view(),
        Command::PrevView => app.prev_view(),
        Command::ShowView(view) => app.view = view,
        Command::StartAutoTest => app.start_auto_test(),
        Command::TogglePause => app.toggle_pause(),
        Command::ResetCurrent => app.reset_current(),
        Command::ResetAll => app.reset_all(),
        Command::Export => {
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
        Command::ToggleShortcuts => app.toggle_shortcuts(),
        Command::ToggleTheme => app.toggle_theme(),
        Command::SendVirtualKeys => app.virtual_test.request_virtual_test(),
        Command::AddOemMapping => app.add_oem_mapping_for_last_unknown(),
        Command::CycleFnMode => app.cycle_fn_mode(),
        Command::ClearOemMappings => app.clear_oem_mappings(),
        Command::SettingsBack => app.view = AppView::Dashboard,
        Command::SettingsUp => {
            if app.settings_selected > 0 {
                app.settings_selected -= 1;
            }
        }
        Command::SettingsDown => {
            let item_count = app.settings_items().len();
            if app.settings_selected + 1 < item_count {
                app.settings_selected += 1;
            }
        }
        Command::SettingsIncrease => app.adjust_setting(true),
        Command::SettingsDecrease => app.adjust_setting(false),
        Command::SettingsSave => app.save_config(),
    }
    // Entering a key-test view: say how commands work there
    if app.view != view_before && is_key_test_view(app.view) {
        let hint = if app.view == AppView::Shortcuts {
            SHORTCUTS_VIEW_HINT
        } else {
            TEST_VIEW_HINT
        };
        app.set_status(hint.to_string());
    }
}

/// Main-loop frame interval for a refresh rate read from the config.
///
/// The config file is hand-editable, so 0 Hz (division by zero) and absurd
/// rates (a busy loop) are clamped to 1-1000 Hz.
fn frame_interval(refresh_rate_hz: u32) -> Duration {
    Duration::from_micros(1_000_000 / u64::from(refresh_rate_hz.clamp(1, 1000)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: CtKeyCode, modifiers: KeyModifiers) -> CtKeyEvent {
        CtKeyEvent::new_with_kind(code, modifiers, KeyEventKind::Press)
    }

    fn plain(c: char) -> CtKeyEvent {
        press(CtKeyCode::Char(c), KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> CtKeyEvent {
        press(CtKeyCode::Char(c), KeyModifiers::CONTROL)
    }

    /// Legacy terminals and Windows: Shift+a arrives as 'A' with SHIFT
    fn legacy_shift(c: char) -> CtKeyEvent {
        press(CtKeyCode::Char(c), KeyModifiers::SHIFT)
    }

    /// Kitty keyboard protocol: Shift+a arrives as the base key 'a' with SHIFT
    fn kitty_shift(c: char) -> CtKeyEvent {
        press(CtKeyCode::Char(c), KeyModifiers::SHIFT)
    }

    fn cmd(key: CtKeyEvent, view: AppView) -> Command {
        command_for_key(&key, view, false, true)
    }

    const TEST_VIEWS: [AppView; 8] = [
        AppView::PollingRate,
        AppView::HoldRelease,
        AppView::Stickiness,
        AppView::Rollover,
        AppView::Latency,
        AppView::Shortcuts,
        AppView::Virtual,
        AppView::OemKeys,
    ];

    #[test]
    fn mashing_keys_in_test_views_never_triggers_commands() {
        // Every printable key a user may hit while testing, plain and shifted
        let mut keys: Vec<CtKeyEvent> = "abcdefghijklmnopqrstuvwxyz0123456789 -=[];',./`\\"
            .chars()
            .flat_map(|c| [plain(c), kitty_shift(c)])
            .collect();
        keys.extend("ASRE?!@QZ".chars().map(legacy_shift));
        keys.extend(
            [
                CtKeyCode::Enter,
                CtKeyCode::Backspace,
                CtKeyCode::Up,
                CtKeyCode::F(5),
            ]
            .map(|k| press(k, KeyModifiers::NONE)),
        );
        for view in TEST_VIEWS {
            for key in &keys {
                let command = cmd(*key, view);
                let view_action = matches!(
                    (view, key.code),
                    (AppView::Virtual, CtKeyCode::Char('v'))
                        | (AppView::OemKeys, CtKeyCode::Char('a' | 'f' | 'c'))
                ) && key.modifiers.is_empty();
                assert!(
                    command == Command::None || view_action,
                    "{:?} in {:?} gave {:?}",
                    key,
                    view,
                    command
                );
            }
        }
    }

    #[test]
    fn test_views_keep_navigation_quit_and_ctrl_commands() {
        for view in TEST_VIEWS {
            assert_eq!(
                cmd(press(CtKeyCode::Esc, KeyModifiers::NONE), view),
                Command::Quit
            );
            assert_eq!(cmd(ctrl('c'), view), Command::Quit);
            assert_eq!(
                cmd(press(CtKeyCode::Tab, KeyModifiers::NONE), view),
                Command::NextView
            );
            assert_eq!(
                cmd(press(CtKeyCode::BackTab, KeyModifiers::SHIFT), view),
                Command::PrevView
            );
            assert_eq!(
                cmd(press(CtKeyCode::Tab, KeyModifiers::SHIFT), view),
                Command::PrevView
            );
            if view != AppView::Shortcuts {
                assert_eq!(cmd(ctrl('p'), view), Command::TogglePause);
                assert_eq!(cmd(ctrl('r'), view), Command::ResetCurrent);
                assert_eq!(cmd(ctrl('e'), view), Command::Export);
            }
        }
        assert_eq!(cmd(plain('v'), AppView::Virtual), Command::SendVirtualKeys);
        assert_eq!(cmd(plain('a'), AppView::OemKeys), Command::AddOemMapping);
        assert_eq!(cmd(plain('f'), AppView::OemKeys), Command::CycleFnMode);
        assert_eq!(cmd(plain('c'), AppView::OemKeys), Command::ClearOemMappings);
    }

    #[test]
    fn ctrl_combos_are_test_input_in_the_shortcuts_view() {
        // Testing Ctrl+R / Ctrl+P / Ctrl+E there must not reset the recorded
        // shortcuts, pause the capture or write a report.
        for c in "abcdefghijklmnopqrstuvwxyz".chars().filter(|c| *c != 'c') {
            assert_eq!(cmd(ctrl(c), AppView::Shortcuts), Command::None, "Ctrl+{c}");
        }
        assert_eq!(cmd(ctrl('c'), AppView::Shortcuts), Command::Quit);
        let mut app = App::default();
        apply_command(&mut app, Command::ShowView(AppView::Shortcuts));
        assert_eq!(app.get_status(), Some(SHORTCUTS_VIEW_HINT));
    }

    #[test]
    fn overview_screens_keep_single_key_commands() {
        for view in [AppView::Dashboard, AppView::AutoTest, AppView::Help] {
            assert_eq!(cmd(plain('q'), view), Command::Quit);
            assert_eq!(cmd(plain(' '), view), Command::TogglePause);
            assert_eq!(cmd(plain('r'), view), Command::ResetCurrent);
            assert_eq!(cmd(plain('e'), view), Command::Export);
            assert_eq!(cmd(plain('m'), view), Command::ToggleShortcuts);
            assert_eq!(cmd(plain('t'), view), Command::ToggleTheme);
            assert_eq!(cmd(plain('5'), view), Command::ShowView(AppView::Rollover));
            assert_eq!(cmd(plain('1'), view), Command::ShowView(AppView::Dashboard));
            assert_eq!(cmd(plain('0'), view), Command::ShowView(AppView::AutoTest));
            // View actions belong to their views only
            assert_eq!(cmd(plain('v'), view), Command::None);
            assert_eq!(cmd(plain('a'), view), Command::None);
        }
        // Number keys freed with 'm'
        assert_eq!(
            command_for_key(&plain('5'), AppView::Dashboard, false, false),
            Command::None
        );
    }

    #[test]
    fn shifted_commands_work_with_legacy_and_kitty_terminals() {
        // (legacy/Windows character, kitty base key) for the same keystroke
        for (legacy, kitty, expected) in [
            ('A', 'a', Command::StartAutoTest),
            ('S', 's', Command::ShowView(AppView::Settings)),
            ('R', 'r', Command::ResetAll),
            ('?', '/', Command::ShowView(AppView::Help)),
        ] {
            assert_eq!(cmd(legacy_shift(legacy), AppView::Dashboard), expected);
            assert_eq!(cmd(kitty_shift(kitty), AppView::Dashboard), expected);
            assert_eq!(cmd(kitty_shift(kitty), AppView::AutoTest), expected);
        }
        // Ctrl+Shift chords are still Ctrl chords
        assert_eq!(
            cmd(
                press(
                    CtKeyCode::Char('c'),
                    KeyModifiers::CONTROL | KeyModifiers::SHIFT
                ),
                AppView::Rollover
            ),
            Command::Quit
        );
    }

    #[test]
    fn running_auto_test_captures_every_key() {
        let run = |key: CtKeyEvent| command_for_key(&key, AppView::AutoTest, true, true);
        for key in [
            plain('q'),
            plain(' '),
            plain('A'),
            legacy_shift('A'),
            press(CtKeyCode::Esc, KeyModifiers::NONE),
            press(CtKeyCode::Tab, KeyModifiers::NONE),
            ctrl('p'),
            ctrl('r'),
            ctrl('e'),
        ] {
            assert_eq!(run(key), Command::None, "{:?}", key);
        }
        assert_eq!(run(ctrl('n')), Command::SkipAutoStep);
        assert_eq!(run(ctrl('c')), Command::AbortAutoTest);
    }

    #[test]
    fn only_key_presses_are_commands() {
        for kind in [KeyEventKind::Repeat, KeyEventKind::Release] {
            let key = CtKeyEvent::new_with_kind(CtKeyCode::Esc, KeyModifiers::NONE, kind);
            assert_eq!(cmd(key, AppView::Dashboard), Command::None);
        }
    }

    #[test]
    fn altgr_and_unmapped_characters_are_never_commands() {
        // AltGr is Ctrl+Alt on Windows; Ctrl+Q is not a command
        let altgr = press(
            CtKeyCode::Char('e'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        );
        assert_eq!(cmd(altgr, AppView::Rollover), Command::None);
        assert_eq!(cmd(ctrl('q'), AppView::Dashboard), Command::None);
        for c in ['é', 'ß', 'ü', '€', 'ñ'] {
            for view in [AppView::Dashboard, AppView::Rollover, AppView::Settings] {
                assert_eq!(cmd(plain(c), view), Command::None);
                assert_eq!(cmd(kitty_shift(c), view), Command::None);
            }
        }
    }

    #[test]
    fn settings_view_keys() {
        let v = AppView::Settings;
        assert_eq!(cmd(plain('q'), v), Command::SettingsBack);
        assert_eq!(
            cmd(press(CtKeyCode::Esc, KeyModifiers::NONE), v),
            Command::SettingsBack
        );
        assert_eq!(cmd(plain('s'), v), Command::SettingsSave);
        assert_eq!(
            cmd(press(CtKeyCode::Right, KeyModifiers::NONE), v),
            Command::SettingsIncrease
        );
        assert_eq!(
            cmd(press(CtKeyCode::Tab, KeyModifiers::NONE), v),
            Command::NextView
        );
        assert_eq!(cmd(ctrl('c'), v), Command::Quit);
    }

    #[test]
    fn entering_a_test_view_shows_the_command_hint() {
        let mut app = App::default();
        apply_command(&mut app, Command::ShowView(AppView::Rollover));
        assert_eq!(app.get_status(), Some(TEST_VIEW_HINT));
        // Mashing keys there leaves the app running and data intact
        app.process_event(&keyboard_testkit::keyboard::KeyEvent::new(
            keyboard_testkit::keyboard::KeyCode(16),
            keyboard_testkit::keyboard::KeyEventType::Press,
            std::time::Instant::now(),
            0,
        ));
        for c in "qre ".chars() {
            let command = cmd(plain(c), app.view);
            apply_command(&mut app, command);
        }
        assert_eq!(app.state, AppState::Running);
        assert_eq!(app.total_events, 1);
    }

    #[test]
    fn frame_interval_follows_refresh_rate_and_survives_bad_values() {
        assert_eq!(frame_interval(60), Duration::from_micros(16_666));
        assert_eq!(frame_interval(120), Duration::from_micros(8_333));
        // refresh_rate_hz = 0 in a hand-edited config used to divide by zero
        assert_eq!(frame_interval(0), Duration::from_secs(1));
        assert_eq!(frame_interval(u32::MAX), Duration::from_millis(1));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn one_unplug_of_a_multi_node_keyboard_is_one_disconnect() {
        let t0 = std::time::Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        let mut log = DisconnectLog::default();
        // Keyboard, consumer-control and NKRO nodes vanish together
        log.node_lost(ms(0));
        log.node_lost(ms(3));
        log.node_lost(ms(20));
        assert_eq!(log.count, 1);
        // It comes back and drops out again: a second disconnect
        log.node_found();
        log.node_lost(ms(900));
        assert_eq!(log.count, 2);
        // A later, separate drop without a reconnect seen in between
        log.node_lost(ms(10_000));
        assert_eq!(log.count, 3);
    }

    #[test]
    fn disconnects_are_reported_as_an_issue() {
        assert!(disconnect_finding(0).is_none());
        let finding = disconnect_finding(2).unwrap();
        assert!(finding.is_issue());
        assert!(finding.title.contains("2 time(s)"));
    }
}
