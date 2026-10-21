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
// info!/error! print to stderr, so they are only used outside the TUI (the
// mapper, before the alternate screen is entered); inside it messages go to
// the status line.
macro_rules! info    { ($($arg:tt)*) => { eprintln!("[INFO]  {}", format_args!($($arg)*)) } }
#[cfg_attr(not(target_os = "linux"), allow(unused_macros))]
macro_rules! error   { ($($arg:tt)*) => { eprintln!("[ERROR] {}", format_args!($($arg)*)) } }

use crossterm::{
    cursor,
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
use std::time::{Duration, Instant};

use keyboard_testkit::{
    config::Config,
    keyboard::{crossterm_to_keycode, keymap, KeyCode, KeyEvent, KeyboardListener},
    tests::{Finding, InputSource},
    ui::{
        App, AppState, AppView, HelpPanel, KeyboardVisual, ResultsPanel, SettingsPanel,
        ShortcutOverlay, StatusBar, TabBar,
    },
};

#[cfg(target_os = "linux")]
use keyboard_testkit::keyboard::{evdev_status, EvdevDeviceChange, EvdevListener};

#[cfg(target_os = "linux")]
use keyboard_testkit::mapper;

/// Restore the terminal to its original state after a panic.
fn restore_terminal() {
    let _ = leave_tui(
        &mut std::io::stdout(),
        KEYBOARD_ENHANCED.swap(false, Ordering::SeqCst),
    );
    let _ = disable_raw_mode();
}

/// Leave the TUI screen: pop the keyboard flags (if `enhanced`) while still
/// on the alternate screen, whose flag stack they were pushed on, then
/// switch back to the shell's screen and show the cursor ratatui hid.
fn leave_tui(out: &mut impl std::io::Write, enhanced: bool) -> std::io::Result<()> {
    if enhanced {
        execute!(out, PopKeyboardEnhancementFlags)?;
    }
    execute!(out, LeaveAlternateScreen, DisableMouseCapture, cursor::Show)
}

/// Whether kitty keyboard-protocol enhancement flags were pushed (must be popped on exit).
static KEYBOARD_ENHANCED: AtomicBool = AtomicBool::new(false);

/// Try to enable key-release reporting (kitty keyboard protocol).
///
/// Call it on the alternate screen: terminals keep separate flag stacks for
/// the main and alternate screens, so flags pushed before entering it would
/// not apply to the TUI (presses only, no releases) and would be left
/// active on the shell's screen after exit.
///
/// Returns true when the terminal supports it and the flags were pushed.
fn enable_keyboard_enhancement(out: &mut impl std::io::Write) -> bool {
    match crossterm::terminal::supports_keyboard_enhancement() {
        Ok(true) => {
            if push_keyboard_flags(out).is_ok() {
                KEYBOARD_ENHANCED.store(true, Ordering::SeqCst);
                return true;
            }
            false
        }
        _ => false,
    }
}

/// Ask for press/repeat/release events for every key, modifiers included.
fn push_keyboard_flags(out: &mut impl std::io::Write) -> std::io::Result<()> {
    let flags = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
        | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES;
    execute!(out, PushKeyboardEnhancementFlags(flags))
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

/// The `--config <path>` option: a config file other than the default one.
fn config_arg() -> Option<std::path::PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    args.windows(2)
        .find(|w| w[0] == "--config")
        .map(|w| std::path::PathBuf::from(&w[1]))
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

            let config = config_arg();
            if let Err(e) =
                mapper::run_mapper(preset.as_deref(), device, config.as_deref(), &[], running)
            {
                error!("Mapper error: {}", e);
                // Its own exit status, which the service does not restart on
                if matches!(e, mapper::MapperError::NoMappings) {
                    std::process::exit(mapper::EXIT_NOTHING_TO_MAP);
                }
                return Err(e.into());
            }
            return Ok(());
        }

        #[cfg(target_os = "linux")]
        CliMode::MapperInstall { preset } => {
            info!("Installing key mapper service...");
            if let Err(e) = mapper::install_service(preset.as_deref(), config_arg().as_deref()) {
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
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Ctrl+C is handled via crossterm key events in raw mode.
    // The `running` flag is set to false when the user presses q/Esc.
    let running = Arc::new(AtomicBool::new(true));
    install_signal_handler(running.clone());

    // Run the application; cleanup runs regardless of success or failure
    let result = run_app(&mut terminal, running, auto_start);

    // Cleanup terminal — always runs, even after signal or error
    let left = leave_tui(
        terminal.backend_mut(),
        KEYBOARD_ENHANCED.swap(false, Ordering::SeqCst),
    );
    disable_raw_mode()?;
    left?;

    if let Ok(Some(summary)) = &result {
        println!("\nKeyboard TestKit session complete.");
        println!("Input source: {}", summary.input_source);
        println!("Total events processed: {}", summary.total_events);
        println!("Session duration: {}", summary.elapsed);
        // Same findings as the Dashboard and the exported report (keyboard
        // disconnects included)
        let issues: Vec<&Finding> = summary.findings.iter().filter(|f| f.is_issue()).collect();
        println!("Issues found: {}", issues.len());
        for f in issues {
            println!("  [{}] {}: {}", f.severity.label(), f.category, f.title);
            if !f.detail.is_empty() {
                println!("      {}", f.detail);
            }
        }
        for note in &summary.config_notes {
            println!("Config: {}", note);
        }
    }

    result.map(|_| ())
}

/// What the TUI session reports on exit.
struct SessionOutcome {
    /// Problems with the config file (it could not be used, values clamped)
    config_notes: Vec<String>,
    input_source: &'static str,
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
    println!("  --config <path>               Use this config file (TUI and mapper)");
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
    println!("  # Custom mappings via config file (under sudo, ~ is your own home;");
    println!("  # the installed service is given your config file's path)");
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
    auto_start: bool,
) -> Result<Option<SessionOutcome>> {
    let config_load = match config_arg() {
        Some(path) => Config::load_reporting_from(&path),
        None => Config::load_reporting(),
    };
    let mut app = App::new(config_load.config.clone());

    // Create keyboard event channel
    let (event_tx, event_rx) = mpsc::channel::<KeyEvent>();

    // Create keyboard listener (crossterm-based fallback for non-Linux)
    let mut listener = KeyboardListener::new(event_tx.clone());

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

    // When the terminal is the test input, ask it for key-release events
    // where it supports them (kitty, foot, Ghostty, Alacritty 0.13+, WezTerm
    // with enable_kitty_keyboard = true); Windows consoles report releases
    // natively. Evdev already has real releases, so the terminal is left in
    // its normal mode.
    let release_reporting =
        !use_evdev && (cfg!(windows) || enable_keyboard_enhancement(terminal.backend_mut()));
    listener.set_release_reporting(release_reporting);

    let input_source = if use_evdev {
        InputSource::Evdev
    } else if release_reporting {
        InputSource::TerminalWithRelease
    } else {
        InputSource::TerminalPressOnly
    };
    app.set_input_source(input_source);
    // After the evdev status: a config file that could not be used must be seen
    app.set_config_load(&config_load);

    if auto_start {
        app.start_auto_test();
    }
    let mut held = HeldKeyFilter::default();

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
                report_device_changes(&mut app, evdev);
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
                    handle_terminal_key(&mut app, &mut listener, &mut held, use_evdev, key);
                    // The tests see each keystroke before the next key's
                    // command is decided: an Enter read in the same burst
                    // as other keys is typing, not a pause (AutoKeys::Ended)
                    while let Ok(key_event) = event_rx.try_recv() {
                        app.process_event(&key_event);
                    }
                }
                if app.state == AppState::Quitting || !event::poll(Duration::ZERO)? {
                    break;
                }
            }
        }

        // Execute pending virtual key sends. (Nothing may be printed to
        // stderr while the TUI is up: it would land at the hidden cursor,
        // where ratatui never repaints it. Results go to the status line.)
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

    app.refresh_findings();
    Ok(Some(SessionOutcome {
        config_notes: app.config_notes().to_vec(),
        input_source: app.input_source.name(),
        total_events: app.total_events,
        elapsed: app.elapsed_formatted(),
        findings: app.findings.clone(),
    }))
}

/// Status shown while evdev has no keyboard to read.
#[cfg(target_os = "linux")]
const NO_KEYBOARD_STATUS: &str =
    "No keyboard connected - plug it in or reconnect it, it is picked up automatically";

/// Make keyboards that disappear or come back visible in the status line,
/// and record disconnects as a finding (Dashboard, report, exit summary).
#[cfg(target_os = "linux")]
fn report_device_changes(app: &mut App, evdev: &mut EvdevListener) {
    let mut batch = DeviceBatch::default();
    for change in evdev.take_device_changes() {
        apply_device_change(app, change, evdev.device_count(), &mut batch);
    }
    // Keep saying so while nothing is connected (other messages expire)
    if evdev.device_count() == 0 && app.get_status().is_none() {
        app.set_status(NO_KEYBOARD_STATUS.to_string());
    }
}

/// What the device changes read in one poll did so far: unplugging one
/// keyboard removes several nodes at once.
#[cfg(target_os = "linux")]
#[derive(Debug, Default)]
struct DeviceBatch {
    /// A keyboard that was typed on went away
    keyboard_lost: bool,
    /// That stopped the auto test
    stopped: bool,
}

/// Handle one device appearing or disappearing; `devices_left` are still open.
///
/// A keyboard that was typed on going away is a disconnect: a finding, and
/// it stops a running auto test, whose steps would otherwise time out and
/// blame the missing input on dead keys. A node nothing was typed on (a
/// YubiKey, the keyboard interface of a mouse) is only mentioned, unless a
/// keyboard went away with it.
#[cfg(target_os = "linux")]
fn apply_device_change(
    app: &mut App,
    change: EvdevDeviceChange,
    devices_left: usize,
    batch: &mut DeviceBatch,
) {
    match change {
        EvdevDeviceChange::Disconnected {
            name, typed: true, ..
        } => {
            app.keyboard_disconnected(&name);
            batch.keyboard_lost = true;
            if app.auto_test_running() {
                app.abort_auto_test();
                batch.stopped = true;
            }
            let left = match devices_left {
                0 => "no keyboard left".to_string(),
                n => format!("{} device(s) left", n),
            };
            app.set_status(format!(
                "Keyboard disconnected: {} ({}){}",
                name,
                left,
                if batch.stopped {
                    " - auto test stopped"
                } else {
                    ""
                }
            ));
        }
        EvdevDeviceChange::Disconnected { name, .. } => {
            if !batch.keyboard_lost {
                app.set_status(format!(
                    "Input device removed: {} (nothing was typed on it)",
                    name
                ));
            }
        }
        EvdevDeviceChange::Connected { name, .. } => {
            app.keyboard_connected();
            app.set_status(format!("Keyboard connected: {}", name));
        }
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
    DismissAutoResult,
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

impl Command {
    /// Whether the command may fire again while its key is held: moving
    /// between views or settings values, which is what holding Tab or an
    /// arrow key is for, and dismissing a run's result, which does nothing
    /// once done (Enter pressed again and again on a terminal without
    /// releases looks held). Everything else changes state once per
    /// keystroke.
    fn repeats_safely(self) -> bool {
        matches!(
            self,
            Command::None
                | Command::DismissAutoResult
                | Command::NextView
                | Command::PrevView
                | Command::ShowView(_)
                | Command::SettingsUp
                | Command::SettingsDown
                | Command::SettingsIncrease
                | Command::SettingsDecrease
        )
    }
}

/// How much of the keyboard the auto test claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutoKeys {
    /// No run in progress: the usual commands
    Free,
    /// A run is in progress: every key is test input but Ctrl+N and Ctrl+C
    Running,
    /// A run ended and its result is still up (see
    /// `App::auto_result_pending`): plain keys are still test input and only
    /// Ctrl chords are commands, except that Enter dismisses the result once
    /// the keyboard has been `quiet` (see `App::auto_test_just_ended`)
    Ended { quiet: bool },
}

impl AutoKeys {
    fn of(app: &App, now: Instant) -> Self {
        if app.auto_test_running() {
            AutoKeys::Running
        } else if app.auto_result_pending() {
            AutoKeys::Ended {
                quiet: !app.auto_test_just_ended_at(now),
            }
        } else {
            AutoKeys::Free
        }
    }
}

/// Reports of a held key come this soon after the previous report of it:
/// the first autorepeat after the repeat delay (Windows and GNOME 500 ms,
/// KDE 600 ms, X11 660 ms, plus read latency), then one every 30-50 ms.
const HELD_KEY_WINDOW: Duration = Duration::from_millis(750);

/// Recognises a held command key. Legacy terminals and the Windows console
/// report autorepeat as more presses of the same key (only the kitty
/// protocol marks them as repeats), so holding Ctrl+N would skip one step
/// per repeat and holding Ctrl+C would abort the auto test and then quit.
///
/// A press identical to the previous report (same key and modifiers) within
/// [`HELD_KEY_WINDOW`] of it, with no release of that key in between, is
/// that key still held. The cost, on terminals without release events, is
/// that pressing the same command key twice within 750 ms counts once.
#[derive(Debug, Default)]
struct HeldKeyFilter {
    /// The latest report of a key and when it arrived
    last: Option<(CtKeyCode, KeyModifiers, Instant)>,
}

impl HeldKeyFilter {
    /// Note `key` arriving at `now`; returns whether it is a held key
    /// reporting again rather than a new keystroke.
    fn is_held(&mut self, key: &CtKeyEvent, now: Instant) -> bool {
        let same = |(code, modifiers, _): &(CtKeyCode, KeyModifiers, Instant)| {
            *code == key.code && *modifiers == key.modifiers
        };
        match key.kind {
            KeyEventKind::Release => {
                // The key came up, whatever is still held: the Ctrl of Ctrl+N
                // is often let go first ('n' comes up alone), and Shift+R
                // comes up as 'r' once Shift is up
                if self
                    .last
                    .as_ref()
                    .is_some_and(|(code, _, _)| same_key(*code, key.code))
                {
                    self.last = None;
                }
                false
            }
            KeyEventKind::Repeat => {
                self.last = Some((key.code, key.modifiers, now));
                true
            }
            KeyEventKind::Press => {
                let held = self.last.as_ref().is_some_and(|last| {
                    same(last) && now.saturating_duration_since(last.2) < HELD_KEY_WINDOW
                });
                self.last = Some((key.code, key.modifiers, now));
                held
            }
        }
    }
}

/// Whether two key codes are the same key, whatever the case of a letter.
fn same_key(a: CtKeyCode, b: CtKeyCode) -> bool {
    match (a, b) {
        (CtKeyCode::Char(a), CtKeyCode::Char(b)) => a.to_lowercase().eq(b.to_lowercase()),
        _ => a == b,
    }
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
///   step) and Ctrl+C (abort), and after it ends plain keys stay test input
///   until the user pauses and presses Enter (see [`AutoKeys::Ended`]);
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
    auto: AutoKeys,
    shortcuts_enabled: bool,
) -> Command {
    // Controls only fire on the initial press, never on repeat or release
    // (repeats a terminal reports as presses: see `HeldKeyFilter`).
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
        return if auto == AutoKeys::Running {
            Command::AbortAutoTest
        } else {
            Command::Quit
        };
    }

    // While the auto test runs, every key is test input: only the two
    // Ctrl chords are interpreted, so 'q', 'r', Esc... can be verified like
    // any other key.
    if auto == AutoKeys::Running {
        return if ctrl_char == Some('n') {
            Command::SkipAutoStep
        } else {
            Command::None
        };
    }
    // The run ended, maybe under the user's fingers (the rapid-tap step ends
    // on a timer, a disconnect stops a sweep the user may go on with after a
    // pause): plain keys are not commands until the user says so with Enter,
    // and an Enter typed along with other keys (a sweep has one) does not.
    if let AutoKeys::Ended { quiet } = auto {
        if !ctrl {
            return if quiet && key.code == CtKeyCode::Enter {
                Command::DismissAutoResult
            } else {
                Command::None
            };
        }
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

/// The command a terminal key event asks for, given what came before it.
fn command_for_event(
    key: &CtKeyEvent,
    app: &App,
    held: &mut HeldKeyFilter,
    now: Instant,
) -> Command {
    let command = command_for_key(key, app.view, AutoKeys::of(app, now), app.shortcuts_enabled);
    if held.is_held(key, now) && !command.repeats_safely() {
        Command::None
    } else {
        command
    }
}

/// Feed a terminal key event to the test pipeline and handle UI controls.
fn handle_terminal_key(
    app: &mut App,
    listener: &mut KeyboardListener,
    held: &mut HeldKeyFilter,
    use_evdev: bool,
    key: CtKeyEvent,
) {
    let command = command_for_event(&key, app, held, Instant::now());

    // When evdev is not in use, the terminal is the test input. Presses,
    // repeats and releases each mean something different to the tests.
    // The keystroke that starts the auto test must not be counted by the
    // test it starts (it would show up as phantom input in the idle step),
    // and neither may its autorepeat and release when the key is held a
    // little long. (With evdev its kernel copy predates the run.)
    if !use_evdev {
        match key.kind {
            KeyEventKind::Press if command == Command::StartAutoTest => {
                listener.swallow_press(key.code);
            }
            KeyEventKind::Press => {
                listener.send_press_with(key.code, key.modifiers);
            }
            KeyEventKind::Repeat => listener.send_repeat(key.code),
            KeyEventKind::Release => {
                listener.send_release_with(key.code, key.modifiers);
            }
        }
    }

    // A reset clears the tests under the keystroke that asked for it: its
    // keys (Ctrl and R) are not rollover or stuck-key input for them
    match command {
        Command::ResetCurrent => app.reset_current_on_chord(&keystroke_keys(&key)),
        Command::ResetAll => app.reset_all_on_chord(&keystroke_keys(&key)),
        _ => apply_command(app, command),
    }
}

/// The keys of a command keystroke: the key itself and both keys of each
/// modifier it was typed with (the terminal does not say which one was
/// held). Legacy terminals send Shift+r as 'R'.
fn keystroke_keys(key: &CtKeyEvent) -> Vec<KeyCode> {
    let mut keys = vec![crossterm_to_keycode(key.code)];
    let shift = key.modifiers.contains(KeyModifiers::SHIFT)
        || matches!(key.code, CtKeyCode::Char(c) if c.is_uppercase());
    for (held, pair) in [
        (
            key.modifiers.contains(KeyModifiers::CONTROL),
            [keymap::KEY_LCTRL, keymap::KEY_RCTRL],
        ),
        (shift, [keymap::KEY_LSHIFT, keymap::KEY_RSHIFT]),
        (
            key.modifiers.contains(KeyModifiers::ALT),
            [keymap::KEY_LALT, keymap::KEY_RALT],
        ),
        (
            key.modifiers.contains(KeyModifiers::SUPER),
            [keymap::KEY_LSUPER, keymap::KEY_RSUPER],
        ),
    ] {
        if held {
            keys.extend(pair);
        }
    }
    keys.retain(|k| k.0 != 0);
    keys
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
        Command::DismissAutoResult => app.dismiss_auto_result(),
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
            // The outcome goes to the status line (export_report sets it on
            // success), never to stderr while the TUI is drawn
            if let Err(e) = app.export_report(&filename) {
                app.set_status(format!("Export failed: {}", e));
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
        command_for_key(&key, view, AutoKeys::Free, true)
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
            command_for_key(&plain('5'), AppView::Dashboard, AutoKeys::Free, false),
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
        let run =
            |key: CtKeyEvent| command_for_key(&key, AppView::AutoTest, AutoKeys::Running, true);
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
    fn keys_still_typed_as_the_auto_test_ends_are_not_commands() {
        // Tapping 'r' (or q, Space, a digit...) for the rapid-tap step a
        // moment after it ended must not reset the run, quit or pause, and
        // neither may those keys after a pause
        let enter = press(CtKeyCode::Enter, KeyModifiers::NONE);
        for quiet in [false, true] {
            let ended = |key: CtKeyEvent| {
                command_for_key(&key, AppView::AutoTest, AutoKeys::Ended { quiet }, true)
            };
            for key in [
                plain('q'),
                plain('r'),
                plain(' '),
                plain('e'),
                plain('5'),
                legacy_shift('R'),
                legacy_shift('A'),
                press(CtKeyCode::Esc, KeyModifiers::NONE),
                press(CtKeyCode::Tab, KeyModifiers::NONE),
            ] {
                assert_eq!(ended(key), Command::None, "{:?}", key);
            }
            // Deliberate Ctrl chords still work (there is nothing left to skip)
            assert_eq!(ended(ctrl('c')), Command::Quit);
            assert_eq!(ended(ctrl('e')), Command::Export);
            assert_eq!(ended(ctrl('r')), Command::ResetCurrent);
            assert_eq!(ended(ctrl('n')), Command::None);
            // Enter typed along with other keys is test input; after a
            // pause it gives the commands back
            let dismiss = if quiet {
                Command::DismissAutoResult
            } else {
                Command::None
            };
            assert_eq!(ended(enter), dismiss);
        }
        // Once the result is dismissed the Auto view's keys are back
        assert_eq!(cmd(plain('r'), AppView::AutoTest), Command::ResetCurrent);
        assert_eq!(cmd(enter, AppView::AutoTest), Command::None);
    }

    #[test]
    fn pausing_after_a_disconnect_stops_the_run_does_not_free_command_keys() {
        // The keyboard drops out mid-sweep and comes back; the user goes on
        // with the sweep after a pause of several seconds
        let mut app = App::default();
        app.set_input_source(InputSource::Evdev);
        app.start_auto_test();
        app.keyboard_disconnected("USB Keyboard");
        app.abort_auto_test();
        let ended = app.auto_test.finished_at().unwrap();
        let at = |ms: u64| ended + Duration::from_millis(ms);
        let mut held = HeldKeyFilter::default();
        for (ms, key) in [
            (6_000, plain('q')),
            (6_400, plain('r')),
            (6_900, legacy_shift('R')),
            (7_300, plain(' ')),
            (7_800, press(CtKeyCode::Esc, KeyModifiers::NONE)),
            (8_200, press(CtKeyCode::Tab, KeyModifiers::NONE)),
        ] {
            let command = command_for_event(&key, &app, &mut held, at(ms));
            assert_eq!(command, Command::None, "{key:?} {ms} ms after the stop");
            apply_command(&mut app, command);
            // The evdev copy of the key, read in the same frame
            app.process_event(&KeyEvent::new(
                crossterm_to_keycode(key.code),
                keyboard_testkit::keyboard::KeyEventType::Press,
                at(ms),
                0,
            ));
        }
        // An Enter typed in the sweep, right after other keys, is test input
        let enter = press(CtKeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(
            command_for_event(&enter, &app, &mut held, at(8_700)),
            Command::None
        );
        assert!(app.auto_result_pending());

        // Enter after a pause gives the commands back
        let command = command_for_event(&enter, &app, &mut held, at(12_000));
        assert_eq!(command, Command::DismissAutoResult);
        apply_command(&mut app, command);
        assert!(!app.auto_result_pending());
        let command = command_for_event(&plain('q'), &app, &mut held, at(13_000));
        assert_eq!(command, Command::Quit);
    }

    #[test]
    fn enter_pressed_again_and_again_dismisses_the_result_once_quiet() {
        // A press-only terminal (no releases): the user taps on as the
        // rapid-tap step ends, then presses Enter every 600 ms until the
        // commands come back. Enter pressed again that soon looks held.
        let mut app = App::default();
        app.set_input_source(InputSource::TerminalPressOnly);
        app.start_auto_test();
        app.abort_auto_test();
        let ended = app.auto_test.finished_at().unwrap();
        let at = |ms: u64| ended + Duration::from_millis(ms);
        app.process_event(&KeyEvent::new(
            KeyCode(30),
            keyboard_testkit::keyboard::KeyEventType::Press,
            at(500),
            0,
        ));
        let enter = press(CtKeyCode::Enter, KeyModifiers::NONE);
        let mut held = HeldKeyFilter::default();
        let mut commands = Vec::new();
        for ms in (1_500..4_000).step_by(600) {
            let command = command_for_event(&enter, &app, &mut held, at(ms));
            apply_command(&mut app, command);
            commands.push((ms, command));
        }
        assert_eq!(
            commands,
            vec![
                (1_500, Command::None),
                (2_100, Command::None),
                (2_700, Command::DismissAutoResult),
                (3_300, Command::None),
                (3_900, Command::None),
            ]
        );
        assert!(!app.auto_result_pending());
    }

    /// Commands for `keys`, each `(ms after the first, key)`, fed to `app`
    /// the way the main loop does, applying each command as it comes.
    fn commands_for(app: &mut App, keys: &[(u64, CtKeyEvent)]) -> Vec<Command> {
        let mut held = HeldKeyFilter::default();
        let t0 = Instant::now();
        keys.iter()
            .map(|(ms, key)| {
                let command =
                    command_for_event(key, app, &mut held, t0 + Duration::from_millis(*ms));
                apply_command(app, command);
                command
            })
            .collect()
    }

    /// A key held down in a terminal without release events: the press, then
    /// autorepeat after 500 ms every 33 ms until `until_ms`.
    fn held_down(key: CtKeyEvent, until_ms: u64) -> Vec<(u64, CtKeyEvent)> {
        let mut reports = vec![(0, key)];
        reports.extend((500..=until_ms).step_by(33).map(|ms| (ms, key)));
        reports
    }

    #[test]
    fn holding_a_command_key_fires_it_once() {
        // Ctrl+N held for 0.7 s during the auto test skips one step, not all
        let mut app = App::default();
        app.start_auto_test();
        let commands = commands_for(&mut app, &held_down(ctrl('n'), 700));
        assert_eq!(
            commands
                .iter()
                .filter(|c| **c == Command::SkipAutoStep)
                .count(),
            1
        );
        assert!(app.auto_test_running());

        // Ctrl+C held a little long aborts the run but does not then quit
        let commands = commands_for(&mut app, &held_down(ctrl('c'), 700));
        assert_eq!(commands[0], Command::AbortAutoTest);
        assert!(
            commands[1..].iter().all(|c| *c == Command::None),
            "{commands:?}"
        );
        assert_eq!(app.state, AppState::Running);

        // Space held on the Dashboard pauses once, and stays paused
        let mut app = App::default();
        commands_for(&mut app, &held_down(plain(' '), 900));
        assert_eq!(app.state, AppState::Paused);

        // Esc held in Settings goes back, and does not quit from the Dashboard
        let mut app = App::default();
        app.view = AppView::Settings;
        commands_for(
            &mut app,
            &held_down(press(CtKeyCode::Esc, KeyModifiers::NONE), 900),
        );
        assert_eq!(app.view, AppView::Dashboard);
        assert_eq!(app.state, AppState::Running);
    }

    #[test]
    fn separate_presses_and_moving_keys_still_work() {
        // Two separate Ctrl+N presses skip two steps
        let mut app = App::default();
        app.start_auto_test();
        let commands = commands_for(&mut app, &[(0, ctrl('n')), (1200, ctrl('n'))]);
        assert_eq!(commands, vec![Command::SkipAutoStep; 2]);

        // A reported release ends the keystroke (kitty, Windows console), so
        // a quick second press is a new one
        let release = CtKeyEvent::new_with_kind(
            CtKeyCode::Char(' '),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );
        let mut app = App::default();
        commands_for(
            &mut app,
            &[(0, plain(' ')), (90, release), (200, plain(' '))],
        );
        assert_eq!(app.state, AppState::Running);

        // The release ends the keystroke also when it comes after the
        // modifier was let go (kitty, Windows console): Ctrl+N up as a
        // plain 'n', Shift+R up as 'r'. Two quick Ctrl+N skip two steps.
        let up = |c| {
            CtKeyEvent::new_with_kind(
                CtKeyCode::Char(c),
                KeyModifiers::NONE,
                KeyEventKind::Release,
            )
        };
        let mut app = App::default();
        app.start_auto_test();
        let commands = commands_for(
            &mut app,
            &[(0, ctrl('n')), (120, up('n')), (350, ctrl('n'))],
        );
        assert_eq!(
            commands,
            vec![Command::SkipAutoStep, Command::None, Command::SkipAutoStep]
        );
        let mut app = App::default();
        let commands = commands_for(
            &mut app,
            &[
                (0, legacy_shift('R')),
                (100, up('r')),
                (300, legacy_shift('R')),
            ],
        );
        assert_eq!(
            commands,
            vec![Command::ResetAll, Command::None, Command::ResetAll]
        );

        // Holding Tab or an arrow key keeps moving
        let mut app = App::default();
        let commands = commands_for(
            &mut app,
            &held_down(press(CtKeyCode::Tab, KeyModifiers::NONE), 600),
        );
        assert!(commands.iter().all(|c| *c == Command::NextView));
        assert_eq!(
            app.view,
            AppView::from_index(commands.len() % AppView::all().len())
        );
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

    /// Feed `keys` to a fresh app through the terminal path (no evdev), the
    /// way the main loop does, then let the idle step of the auto test the
    /// first one started run its course.
    fn idle_findings_after(source: InputSource, keys: &[CtKeyEvent]) -> Vec<Finding> {
        let (tx, rx) = mpsc::channel();
        let mut listener = KeyboardListener::new(tx);
        listener.set_release_reporting(source.has_release_events());
        let mut app = App::default();
        app.set_input_source(source);
        let mut held = HeldKeyFilter::default();
        for key in keys {
            handle_terminal_key(&mut app, &mut listener, &mut held, false, *key);
            listener.poll();
            while let Ok(event) = rx.try_recv() {
                app.process_event(&event);
            }
        }
        assert!(app.auto_test_running());
        app.auto_test
            .tick_at(Instant::now() + keyboard_testkit::tests::IDLE_DURATION);
        app.refresh_findings();
        app.findings.clone()
    }

    #[test]
    fn holding_the_start_key_is_not_phantom_input() {
        let shift_a =
            |kind| CtKeyEvent::new_with_kind(CtKeyCode::Char('A'), KeyModifiers::SHIFT, kind);
        let kitty_a =
            |kind| CtKeyEvent::new_with_kind(CtKeyCode::Char('a'), KeyModifiers::SHIFT, kind);
        // Press-only terminal: the held 'A' arrives again as autorepeat
        let press_only = vec![shift_a(KeyEventKind::Press); 12];
        // Kitty protocol: repeat events, then the release
        let mut kitty = vec![kitty_a(KeyEventKind::Press)];
        kitty.extend([kitty_a(KeyEventKind::Repeat); 10]);
        kitty.push(kitty_a(KeyEventKind::Release));
        // Windows console: repeats are presses, then the release
        let mut console = vec![shift_a(KeyEventKind::Press); 10];
        console.push(shift_a(KeyEventKind::Release));
        for (source, keys) in [
            (InputSource::TerminalPressOnly, press_only),
            (InputSource::TerminalWithRelease, kitty),
            (InputSource::TerminalWithRelease, console),
        ] {
            let findings = idle_findings_after(source, &keys);
            assert!(
                !findings.iter().any(|f| f.category == "Phantom input"),
                "{:?}: {:?}",
                source,
                findings
            );
        }

        // A different key pressed during the idle step is still caught
        let keys = [
            shift_a(KeyEventKind::Press),
            press(CtKeyCode::Char('j'), KeyModifiers::NONE),
        ];
        let findings = idle_findings_after(InputSource::TerminalPressOnly, &keys);
        assert!(findings.iter().any(|f| f.category == "Phantom input"));
    }

    /// Escape sequences crossterm writes for the terminal modes involved.
    #[cfg(unix)]
    mod seq {
        pub const ALT_SCREEN_ON: &str = "\x1b[?1049h";
        pub const ALT_SCREEN_OFF: &str = "\x1b[?1049l";
        pub const FLAGS_PUSH: &str = "\x1b[>";
        pub const FLAGS_POP: &str = "\x1b[<1u";
        pub const CURSOR_SHOW: &str = "\x1b[?25h";
    }

    // (Unix only: on Windows crossterm drives the console through its API
    // rather than writing these sequences.)
    #[cfg(unix)]
    #[test]
    fn keyboard_flags_live_on_the_alternate_screen_and_the_cursor_comes_back() {
        // Terminals keep one flag stack per screen: the flags must be pushed
        // after entering the alternate screen and popped before leaving it,
        // or the TUI gets no releases and the shell keeps the flags.
        let mut out = Vec::new();
        execute!(out, EnterAlternateScreen).unwrap();
        push_keyboard_flags(&mut out).unwrap();
        leave_tui(&mut out, true).unwrap();
        let out = String::from_utf8(out).unwrap();
        let at = |s: &str| out.find(s).unwrap_or_else(|| panic!("{s:?} in {out:?}"));
        assert!(at(seq::ALT_SCREEN_ON) < at(seq::FLAGS_PUSH));
        assert!(at(seq::FLAGS_PUSH) < at(seq::FLAGS_POP));
        assert!(at(seq::FLAGS_POP) < at(seq::ALT_SCREEN_OFF));
        // ratatui hides the cursor on every frame; a panic must not leave
        // the shell without one
        assert!(at(seq::ALT_SCREEN_OFF) < at(seq::CURSOR_SHOW));

        // Without pushed flags (evdev input, or a terminal without the
        // protocol) nothing is popped from the shell's stack
        let mut out = Vec::new();
        leave_tui(&mut out, false).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(!out.contains(seq::FLAGS_POP), "{out:?}");
        assert!(out.contains(seq::CURSOR_SHOW), "{out:?}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn only_a_keyboard_that_was_typed_on_counts_as_disconnected() {
        let gone = |name: &str, typed| EvdevDeviceChange::Disconnected {
            path: std::path::PathBuf::from("/dev/input/event9"),
            name: name.to_string(),
            typed,
        };
        // Pulling out a YubiKey mid-sweep leaves the run and the findings alone
        let mut app = App::default();
        app.set_input_source(InputSource::Evdev);
        app.start_auto_test();
        let mut batch = DeviceBatch::default();
        let yubikey = gone("Yubico YubiKey OTP+FIDO+CCID", false);
        apply_device_change(&mut app, yubikey, 2, &mut batch);
        assert!(app.auto_test_running());
        assert_eq!(app.keyboard_disconnects(), 0);
        assert!(app.get_status().unwrap().contains("YubiKey"));

        // The keyboard being typed on going away (with its media-key node,
        // never typed on, in the same poll) stops the run and is a finding
        let mut batch = DeviceBatch::default();
        apply_device_change(&mut app, gone("USB Keyboard", true), 2, &mut batch);
        let media = gone("USB Keyboard Consumer Control", false);
        apply_device_change(&mut app, media, 1, &mut batch);
        assert!(!app.auto_test_running());
        assert_eq!(app.keyboard_disconnects(), 1);
        assert!(app.findings.iter().any(|f| f.category == "Connection"));
        let status = app.get_status().unwrap();
        assert!(
            status.starts_with("Keyboard disconnected: USB Keyboard"),
            "{status}"
        );
        assert!(status.ends_with("auto test stopped"), "{status}");
    }

    #[test]
    fn aborting_the_idle_check_with_ctrl_c_is_not_phantom_input() {
        use keyboard_testkit::keyboard::{KeyCode, KeyEventType};
        let issues = |app: &App| -> Vec<&'static str> {
            app.findings
                .iter()
                .filter(|f| f.is_issue())
                .map(|f| f.category)
                .collect()
        };
        // Evdev: the Ctrl press is read from the device a frame or more
        // before the terminal delivers Ctrl+C
        let mut app = App::default();
        app.set_input_source(InputSource::Evdev);
        app.start_auto_test();
        app.process_event(&KeyEvent::new(
            KeyCode(29),
            KeyEventType::Press,
            Instant::now(),
            0,
        ));
        let (tx, _rx) = mpsc::channel();
        let mut listener = KeyboardListener::new(tx);
        let mut held = HeldKeyFilter::default();
        handle_terminal_key(&mut app, &mut listener, &mut held, true, ctrl('c'));
        assert!(!app.auto_test_running());
        assert!(issues(&app).is_empty(), "{:?}", app.findings);

        // Kitty protocol: the bare Ctrl press is reported as a key event
        let ctrl_key = CtKeyCode::Modifier(crossterm::event::ModifierKeyCode::LeftControl);
        let keys = [
            press(CtKeyCode::Char('a'), KeyModifiers::SHIFT),
            press(ctrl_key, KeyModifiers::CONTROL),
            ctrl('c'),
        ];
        let (tx, rx) = mpsc::channel();
        let mut listener = KeyboardListener::new(tx);
        listener.set_release_reporting(true);
        let mut app = App::default();
        app.set_input_source(InputSource::TerminalWithRelease);
        let mut held = HeldKeyFilter::default();
        for key in keys {
            handle_terminal_key(&mut app, &mut listener, &mut held, false, key);
            while let Ok(event) = rx.try_recv() {
                app.process_event(&event);
            }
        }
        assert!(!app.auto_test_running());
        assert!(issues(&app).is_empty(), "{:?}", app.findings);
        // Quitting with Ctrl still held is not a key without a release either
        app.refresh_findings();
        assert!(issues(&app).is_empty(), "{:?}", app.findings);
    }

    /// An evdev key event: kernel time `at`.
    fn evdev(key: u16, kind: keyboard_testkit::keyboard::KeyEventType, at: Instant) -> KeyEvent {
        KeyEvent::new(keyboard_testkit::keyboard::KeyCode(key), kind, at, 0)
    }

    #[test]
    fn the_reset_chord_is_not_counted_as_rollover() {
        use keyboard_testkit::keyboard::KeyEventType::{Press, Release};
        const CTRL: u16 = 29;
        const R: u16 = 19;
        // Evdev: Ctrl is read from the device long before the terminal
        // delivers Ctrl+R; the R press is read either just before the
        // terminal key (`r_first`) or on the next loop iteration
        for r_first in [false, true] {
            let mut app = App::default();
            app.set_input_source(InputSource::Evdev);
            app.view = AppView::Rollover;
            let t0 = Instant::now() - Duration::from_millis(300);
            app.process_event(&evdev(CTRL, Press, t0));
            let r_down = t0 + Duration::from_millis(150);
            if r_first {
                app.process_event(&evdev(R, Press, r_down));
            }
            let (tx, _rx) = mpsc::channel();
            let mut listener = KeyboardListener::new(tx);
            let mut held = HeldKeyFilter::default();
            handle_terminal_key(&mut app, &mut listener, &mut held, true, ctrl('r'));
            if !r_first {
                app.process_event(&evdev(R, Press, r_down));
            }
            let later = Instant::now() + Duration::from_millis(100);
            app.process_event(&evdev(R, Release, later));
            app.process_event(&evdev(CTRL, Release, later));
            assert_eq!(
                app.rollover_test.max_rollover(),
                0,
                "R read first: {r_first}"
            );
            assert_eq!(app.rollover_test.rollover_rating(), "Not tested");
            assert_eq!(app.rollover_test.peak_summary(), "0");

            // The next keys are counted as usual
            let t = Instant::now() + Duration::from_millis(200);
            for key in [30, 31, 32] {
                app.process_event(&evdev(key, Press, t));
            }
            assert_eq!(app.rollover_test.rollover_rating(), "3KRO");
        }

        // Kitty protocol: Ctrl comes as a key of its own; Shift+R (Reset All)
        // on the Dashboard the same way
        let ctrl_key = CtKeyCode::Modifier(crossterm::event::ModifierKeyCode::LeftControl);
        let shift_key = CtKeyCode::Modifier(crossterm::event::ModifierKeyCode::LeftShift);
        let up =
            |code, modifiers| CtKeyEvent::new_with_kind(code, modifiers, KeyEventKind::Release);
        for (view, modifier, modifiers, key) in [
            (AppView::Rollover, ctrl_key, KeyModifiers::CONTROL, 'r'),
            (AppView::Dashboard, shift_key, KeyModifiers::SHIFT, 'r'),
        ] {
            let (tx, rx) = mpsc::channel();
            let mut listener = KeyboardListener::new(tx);
            listener.set_release_reporting(true);
            let mut app = App::default();
            app.set_input_source(InputSource::TerminalWithRelease);
            app.view = view;
            let mut held = HeldKeyFilter::default();
            for event in [
                press(modifier, modifiers),
                press(CtKeyCode::Char(key), modifiers),
                up(CtKeyCode::Char(key), modifiers),
                up(modifier, KeyModifiers::NONE),
            ] {
                handle_terminal_key(&mut app, &mut listener, &mut held, false, event);
                while let Ok(event) = rx.try_recv() {
                    app.process_event(&event);
                }
            }
            assert_eq!(app.rollover_test.max_rollover(), 0, "{view:?}");
            assert_eq!(app.stickiness_test.held_keys().len(), 0, "{view:?}");
        }
    }

    #[test]
    fn keys_held_before_the_reset_chord_are_kept() {
        use keyboard_testkit::keyboard::KeyEventType::Press;
        const CTRL: u16 = 29;
        const J: u16 = 36;
        let reset = |app: &mut App| {
            let (tx, _rx) = mpsc::channel();
            let mut listener = KeyboardListener::new(tx);
            let mut held = HeldKeyFilter::default();
            handle_terminal_key(app, &mut listener, &mut held, true, ctrl('r'));
        };
        let stuck = |app: &App| -> Vec<u16> {
            app.stickiness_test
                .stuck_keys()
                .iter()
                .map(|(k, _)| k.0)
                .collect()
        };
        // A jammed J, then Ctrl+R in the Sticky view to re-check it
        let mut app = App::default();
        app.set_input_source(InputSource::Evdev);
        app.view = AppView::Stickiness;
        let now = Instant::now();
        app.process_event(&evdev(J, Press, now - Duration::from_secs(5)));
        app.process_event(&evdev(CTRL, Press, now - Duration::from_millis(300)));
        reset(&mut app);
        assert_eq!(stuck(&app), vec![J]);

        // A jammed Ctrl: pressing R alone types Ctrl+R. Ctrl has been down
        // past the stuck-key threshold, so it is no part of the keystroke
        let mut app = App::default();
        app.set_input_source(InputSource::Evdev);
        app.view = AppView::Stickiness;
        app.process_event(&evdev(CTRL, Press, now - Duration::from_secs(5)));
        reset(&mut app);
        assert_eq!(stuck(&app), vec![CTRL]);

        // A key held for rollover while Ctrl+R is pressed still counts
        let mut app = App::default();
        app.set_input_source(InputSource::Evdev);
        app.view = AppView::Rollover;
        app.process_event(&evdev(30, Press, now - Duration::from_millis(800)));
        app.process_event(&evdev(CTRL, Press, now - Duration::from_millis(300)));
        reset(&mut app);
        assert_eq!(app.rollover_test.peak_summary(), "1");
    }

    #[test]
    fn frame_interval_follows_refresh_rate_and_survives_bad_values() {
        assert_eq!(frame_interval(60), Duration::from_micros(16_666));
        assert_eq!(frame_interval(120), Duration::from_micros(8_333));
        // refresh_rate_hz = 0 in a hand-edited config used to divide by zero
        assert_eq!(frame_interval(0), Duration::from_secs(1));
        assert_eq!(frame_interval(u32::MAX), Duration::from_millis(1));
    }
}
