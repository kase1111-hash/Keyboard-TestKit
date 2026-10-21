//! Automatic diagnostic test
//!
//! Drives a short guided sequence of steps and then analyses everything the
//! other tests collected to produce a ranked list of [`Finding`]s, so the user
//! does not have to visit every tab and interpret raw numbers.
//!
//! The steps are:
//!
//! 1. **Idle** – hands off the keyboard; any event is a phantom key press,
//!    and a key already held down when the run started shows up as stuck.
//! 2. **Sweep** – press every key of the main block once; keys that never
//!    register are reported as dead, keys that stay down as stuck. Only
//!    keys the input source can report are expected, and only evdev can call
//!    a key dead (a terminal may simply not pass it on).
//! 3. **Hold** – hold one key for about a second; verifies release reporting.
//! 4. **Rollover** – hold as many letter keys as possible; measures N-key rollover.
//!    The step waits for [`GOOD_ROLLOVER`] keys (or its timeout), so a first
//!    half-hearted attempt is not taken as the keyboard's limit.
//! 5. **Rapid tap** – tap one key quickly; switch chatter shows up as bounce
//!    in [`HoldReleaseTest`] (evdev only), and the taps' key events feed the
//!    polling-interval estimate from timestamp quantization.
//!
//! Steps 3 and 4 need real key-release events and are skipped automatically
//! when the input source cannot provide them (see [`InputSource`]).
//!
//! [`AutoTest::analyze`] can also be run at any time, without the guided
//! sequence, to get live findings from whatever has been typed so far.

use super::{
    EventTimingTest, HoldReleaseTest, OemKeyTest, PollingRateTest, ResultStatus, RolloverTest,
    ShortcutTest, StickinessTest, TestResult, VirtualKeyboardTest,
};
use crate::keyboard::layout::{layout_rows, KeyboardLayout};
use crate::keyboard::{
    crossterm_to_keycode, keymap, KeyCode, KeyEvent, KeyEventType, KeyboardState,
};
use crate::tests::virtual_detect::DiagnosticResult;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

// ============================================================================
// Input source
// ============================================================================

/// Where key events come from, which determines what can be measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputSource {
    /// Linux evdev: raw scancodes, kernel timestamps, real releases.
    Evdev,
    /// Terminal that reports key releases (kitty keyboard protocol, Windows console).
    TerminalWithRelease,
    /// Terminal that only reports key presses; releases are synthesized.
    TerminalPressOnly,
}

impl InputSource {
    /// Whether key release events are real (not synthesized on a timer).
    pub fn has_release_events(&self) -> bool {
        !matches!(self, Self::TerminalPressOnly)
    }

    /// Whether timestamps are precise enough for polling-interval analysis.
    pub fn has_precise_timestamps(&self) -> bool {
        matches!(self, Self::Evdev)
    }

    /// Short human-readable name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Evdev => "evdev (raw device)",
            Self::TerminalWithRelease => "terminal (press+release)",
            Self::TerminalPressOnly => "terminal (press only)",
        }
    }

    /// Whether this source can identify `key` at all.
    ///
    /// Evdev sees every scancode. Terminals identify a key by what it types
    /// (see [`crossterm_to_keycode`]), so keys that type nothing (modifiers,
    /// CapsLock) are only reported with the kitty keyboard protocol, and keys
    /// that only exist on ISO/JIS keyboards are never identified. On Windows,
    /// `TerminalWithRelease` is the console API, which drops Shift/Ctrl/Alt and
    /// gives CapsLock and the Windows key no character.
    pub fn can_report(&self, key: KeyCode) -> bool {
        match self {
            Self::Evdev => true,
            Self::TerminalWithRelease => terminal_can_report(key, !cfg!(windows)),
            Self::TerminalPressOnly => terminal_can_report(key, false),
        }
    }
}

/// Whether a terminal can identify `key`; `kitty` is whether it speaks the
/// kitty keyboard protocol (which reports keys that type no character).
fn terminal_can_report(key: KeyCode, kitty: bool) -> bool {
    use crossterm::event::{KeyCode as Ct, ModifierKeyCode as Mk};
    // Everything a legacy terminal can deliver: printable ASCII and the keys
    // with their own escape sequences.
    let mut codes: Vec<Ct> = (' '..='~').map(Ct::Char).collect();
    codes.extend([
        Ct::Esc,
        Ct::Backspace,
        Ct::Tab,
        Ct::Enter,
        Ct::Up,
        Ct::Down,
        Ct::Left,
        Ct::Right,
        Ct::Home,
        Ct::End,
        Ct::PageUp,
        Ct::PageDown,
        Ct::Insert,
        Ct::Delete,
    ]);
    codes.extend((1..=12).map(Ct::F));
    if kitty {
        codes.extend([
            Ct::CapsLock,
            Ct::NumLock,
            Ct::ScrollLock,
            Ct::PrintScreen,
            Ct::Pause,
            Ct::Menu,
        ]);
        codes.extend(
            [
                Mk::LeftShift,
                Mk::RightShift,
                Mk::LeftControl,
                Mk::RightControl,
                Mk::LeftAlt,
                Mk::RightAlt,
                Mk::LeftSuper,
                Mk::RightSuper,
            ]
            .map(Ct::Modifier),
        );
    }
    codes.into_iter().any(|c| crossterm_to_keycode(c) == key)
}

// ============================================================================
// Findings
// ============================================================================

/// How serious a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Informational: nothing wrong, or a limitation of the test setup.
    Info,
    /// Something worth looking at.
    Warning,
    /// A defect that will affect normal use.
    Critical,
}

impl Severity {
    /// Map to the result status used for colouring in the UI.
    pub fn to_status(self) -> ResultStatus {
        match self {
            Self::Info => ResultStatus::Info,
            Self::Warning => ResultStatus::Warning,
            Self::Critical => ResultStatus::Error,
        }
    }

    /// Short label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Critical => "critical",
        }
    }
}

/// A single issue (or notable observation) produced by the analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Severity of the finding
    pub severity: Severity,
    /// Short category, e.g. "Stuck key"
    pub category: &'static str,
    /// One-line summary
    pub title: String,
    /// Supporting detail (which keys, measured values)
    pub detail: String,
    /// What the user can do about it
    pub advice: String,
}

impl Finding {
    fn new(
        severity: Severity,
        category: &'static str,
        title: impl Into<String>,
        detail: impl Into<String>,
        advice: impl Into<String>,
    ) -> Self {
        Self {
            severity,
            category,
            title: title.into(),
            detail: detail.into(),
            advice: advice.into(),
        }
    }

    /// Whether this finding counts as an issue (warning or critical).
    pub fn is_issue(&self) -> bool {
        self.severity >= Severity::Warning
    }

    /// Render as result rows for the results panel.
    pub fn to_results(&self) -> Vec<TestResult> {
        let status = self.severity.to_status();
        let mut rows = vec![TestResult::new(self.category, &self.title, status)];
        if !self.detail.is_empty() {
            rows.push(TestResult::info("", format!("  {}", self.detail)));
        }
        if !self.advice.is_empty() {
            rows.push(TestResult::info("", format!("  → {}", self.advice)));
        }
        rows
    }
}

// ============================================================================
// Steps
// ============================================================================

/// The individual steps of the guided sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    Idle,
    Sweep,
    Hold,
    Rollover,
    RapidTap,
}

impl StepKind {
    /// Title shown in the UI.
    pub fn title(&self) -> &'static str {
        match self {
            Self::Idle => "Idle check",
            Self::Sweep => "Key sweep",
            Self::Hold => "Hold & release",
            Self::Rollover => "Rollover",
            Self::RapidTap => "Rapid tap",
        }
    }

    /// Instruction shown to the user.
    pub fn instruction(&self) -> &'static str {
        match self {
            Self::Idle => "Hands off the keyboard. Do not press anything.",
            Self::Sweep => "Press every key on the main block once (any order).",
            Self::Hold => "Hold a letter key or Space for about one second, then release it.",
            Self::Rollover => {
                "Press and hold as many letter keys as you can at once (aim for 6+), then \
                 release all."
            }
            Self::RapidTap => "Tap one key with one finger as fast as you can for a few seconds.",
        }
    }

    /// How long the step may run before it is considered timed out. The
    /// sweep runs longer while keys keep registering (see [`SWEEP_GRACE`]).
    pub fn timeout(&self) -> Duration {
        match self {
            Self::Idle => IDLE_DURATION,
            Self::Sweep => Duration::from_secs(120),
            Self::Hold => Duration::from_secs(20),
            Self::Rollover => Duration::from_secs(30),
            Self::RapidTap => Duration::from_secs(20),
        }
    }
}

/// How long the idle step listens for phantom input.
pub const IDLE_DURATION: Duration = Duration::from_secs(3);
/// A hold this long (or longer) satisfies the hold step.
pub const MIN_HOLD: Duration = Duration::from_millis(800);
/// How long the rapid-tap step records after the first press.
pub const RAPID_TAP_DURATION: Duration = Duration::from_secs(5);
/// A sweep that is still verifying new keys runs past its timeout until this
/// long after the latest one, so a slow but steady user is not cut off and
/// told the keys they had not reached yet are dead.
pub const SWEEP_GRACE: Duration = Duration::from_secs(30);
/// Rollover considered good (typical gaming/mechanical keyboards). It is also
/// the USB boot-protocol limit, so every keyboard should reach it.
pub const GOOD_ROLLOVER: usize = 6;

/// Keys a healthy keyboard may simply not have or not send: the ISO and JIS
/// extra keys (expected from the configured layout, not the hardware), Right
/// Ctrl (missing on MacBooks and on laptops where the Copilot key replaced it)
/// and the Windows key (missing on some keyboards, and switched off by the
/// game-mode lock of many gaming keyboards).
const KEYS_NOT_ON_EVERY_KEYBOARD: &[KeyCode] = &[
    KeyCode(86), // ISO key left of Z
    KeyCode(92), // Henkan
    KeyCode(93), // Katakana/Hiragana
    KeyCode(94), // Muhenkan
    keymap::KEY_RCTRL,
    keymap::KEY_LSUPER,
];

/// Taps per second between the first and last of `presses` taps spanning `span`.
pub fn tap_rate(presses: u32, span: Duration) -> Option<f64> {
    (presses >= 2 && !span.is_zero()).then(|| (presses - 1) as f64 / span.as_secs_f64())
}

/// Lifecycle state of the auto test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoTestState {
    /// Not started yet
    Idle,
    /// A step is in progress
    Running,
    /// All steps finished
    Complete,
    /// Stopped by the user before finishing
    Aborted,
}

/// How a step ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepOutcome {
    /// Completion criteria met
    Passed,
    /// Ran out of time before the criteria were met
    TimedOut,
    /// Skipped by the user
    Skipped,
    /// Skipped automatically because the input source cannot support it
    Unsupported,
    /// The run was aborted while this step was in progress
    Aborted,
}

/// Borrowed view of everything the analysis looks at.
pub struct AnalysisContext<'a> {
    pub polling: &'a PollingRateTest,
    pub hold_release: &'a HoldReleaseTest,
    pub stickiness: &'a StickinessTest,
    pub rollover: &'a RolloverTest,
    pub event_timing: &'a EventTimingTest,
    pub shortcuts: &'a ShortcutTest,
    pub virtual_detect: &'a VirtualKeyboardTest,
    pub oem_keys: &'a OemKeyTest,
    pub keyboard_state: &'a KeyboardState,
    /// Times the keyboard dropped off (evdev saw its device go away)
    pub keyboard_disconnects: u32,
    /// Names of the devices that went away
    pub disconnected_devices: &'a [String],
}

/// The automatic diagnostic test.
pub struct AutoTest {
    input_source: InputSource,
    state: AutoTestState,
    steps: Vec<StepKind>,
    current: usize,
    step_started: Option<Instant>,
    started_at: Option<Instant>,
    finished_at: Option<Instant>,
    outcomes: Vec<(StepKind, StepOutcome)>,

    // --- observations ---
    /// Events seen during the idle step
    phantom_events: Vec<(KeyCode, KeyEventType)>,
    /// Keys pressed during the idle step (releases of other keys are the tail
    /// of the gesture that started the test, not phantom input)
    idle_pressed: HashSet<KeyCode>,
    /// Keys the caller asked the sweep to check
    requested_keys: Vec<KeyCode>,
    /// The requested keys the input source can report: what the sweep waits for
    expected_keys: Vec<KeyCode>,
    /// Keys pressed during the sweep
    seen_keys: HashSet<KeyCode>,
    /// Whether any key release arrived during the run
    releases_seen: bool,
    /// When the sweep last verified an expected key it had not seen before
    sweep_progress_at: Option<Instant>,
    /// Press timestamps for hold measurement
    press_times: HashMap<KeyCode, Instant>,
    /// Longest completed hold in the hold step
    longest_hold: Option<(KeyCode, Duration)>,
    /// Whether any key was pressed in the hold step
    hold_pressed: bool,
    /// Keys currently down during the rollover step
    rollover_pressed: HashSet<KeyCode>,
    /// Peak simultaneous keys during the rollover step
    rollover_max: usize,
    /// When the rollover peak was last raised
    rollover_peak_at: Option<Instant>,
    /// First press in the rapid-tap step
    rapid_started: Option<Instant>,
    /// Latest press in the rapid-tap step
    rapid_last: Option<Instant>,
    /// Presses recorded in the rapid-tap step
    rapid_presses: u32,
    /// Presses that occurred in any step
    presses_during_run: u64,
}

impl AutoTest {
    /// Create an auto test for the given input source.
    pub fn new(input_source: InputSource) -> Self {
        let mut test = Self {
            input_source,
            state: AutoTestState::Idle,
            steps: Vec::new(),
            current: 0,
            step_started: None,
            started_at: None,
            finished_at: None,
            outcomes: Vec::new(),
            phantom_events: Vec::new(),
            idle_pressed: HashSet::new(),
            requested_keys: expected_keys_for_layout(KeyboardLayout::Ansi),
            expected_keys: Vec::new(),
            seen_keys: HashSet::new(),
            releases_seen: false,
            sweep_progress_at: None,
            press_times: HashMap::new(),
            longest_hold: None,
            hold_pressed: false,
            rollover_pressed: HashSet::new(),
            rollover_max: 0,
            rollover_peak_at: None,
            rapid_started: None,
            rapid_last: None,
            rapid_presses: 0,
            presses_during_run: 0,
        };
        test.filter_expected_keys();
        test
    }

    /// Change the input source (e.g. once the main loop knows what it got).
    pub fn set_input_source(&mut self, source: InputSource) {
        self.input_source = source;
        self.filter_expected_keys();
    }

    /// The input source this test assumes.
    pub fn input_source(&self) -> InputSource {
        self.input_source
    }

    /// Set the keys the sweep step should check. Keys the input source cannot
    /// report are left out of the sweep (see [`AutoTest::unreportable_keys`]).
    pub fn set_expected_keys(&mut self, keys: Vec<KeyCode>) {
        self.requested_keys = keys;
        self.filter_expected_keys();
    }

    fn filter_expected_keys(&mut self) {
        let source = self.input_source;
        self.expected_keys = self
            .requested_keys
            .iter()
            .copied()
            .filter(|k| source.can_report(*k))
            .collect();
    }

    /// Keys the sweep step expects: the requested keys the input source can report.
    pub fn expected_keys(&self) -> &[KeyCode] {
        &self.expected_keys
    }

    /// Requested keys the sweep cannot check because the input source never
    /// reports them (e.g. modifiers through a plain terminal).
    pub fn unreportable_keys(&self) -> Vec<KeyCode> {
        self.requested_keys
            .iter()
            .filter(|k| !self.expected_keys.contains(k))
            .copied()
            .collect()
    }

    /// Keys verified so far in the sweep step.
    pub fn verified_keys(&self) -> &HashSet<KeyCode> {
        &self.seen_keys
    }

    /// Current lifecycle state.
    pub fn state(&self) -> AutoTestState {
        self.state
    }

    /// Whether a step is currently in progress.
    pub fn is_running(&self) -> bool {
        self.state == AutoTestState::Running
    }

    /// Whether the sequence ran to completion.
    pub fn is_complete(&self) -> bool {
        self.state == AutoTestState::Complete
    }

    /// The step currently in progress, if any.
    pub fn current_step(&self) -> Option<StepKind> {
        if self.is_running() {
            self.steps.get(self.current).copied()
        } else {
            None
        }
    }

    /// Steps in this run (empty before the first start).
    pub fn steps(&self) -> &[StepKind] {
        &self.steps
    }

    /// How each finished step ended.
    pub fn outcomes(&self) -> &[(StepKind, StepOutcome)] {
        &self.outcomes
    }

    /// When the run completed or was stopped (None while it runs or before
    /// the first start).
    pub fn finished_at(&self) -> Option<Instant> {
        self.finished_at
    }

    /// Time spent in the current step.
    pub fn step_elapsed(&self) -> Duration {
        self.step_started.map(|t| t.elapsed()).unwrap_or_default()
    }

    /// Overall progress, 0.0 to 1.0.
    pub fn progress(&self) -> f64 {
        match self.state {
            AutoTestState::Idle => 0.0,
            AutoTestState::Complete => 1.0,
            _ => {
                if self.steps.is_empty() {
                    return 0.0;
                }
                let step = self
                    .current_step()
                    .map(|s| self.step_fraction(s))
                    .unwrap_or(0.0);
                ((self.current as f64 + step) / self.steps.len() as f64).clamp(0.0, 1.0)
            }
        }
    }

    /// Progress within the current step, 0.0 to 1.0.
    fn step_fraction(&self, step: StepKind) -> f64 {
        match step {
            StepKind::Idle => {
                (self.step_elapsed().as_secs_f64() / IDLE_DURATION.as_secs_f64()).min(1.0)
            }
            StepKind::Sweep => {
                if self.expected_keys.is_empty() {
                    1.0
                } else {
                    self.sweep_seen_count() as f64 / self.expected_keys.len() as f64
                }
            }
            StepKind::Hold => {
                if self.longest_hold.is_some() {
                    1.0
                } else {
                    0.0
                }
            }
            StepKind::Rollover => (self.rollover_max as f64 / GOOD_ROLLOVER as f64).min(1.0),
            StepKind::RapidTap => self
                .rapid_started
                .map(|t| (t.elapsed().as_secs_f64() / RAPID_TAP_DURATION.as_secs_f64()).min(1.0))
                .unwrap_or(0.0),
        }
    }

    /// Number of expected keys seen in the sweep.
    pub fn sweep_seen_count(&self) -> usize {
        self.expected_keys
            .iter()
            .filter(|k| self.seen_keys.contains(k))
            .count()
    }

    /// Expected keys not yet seen in the sweep.
    pub fn sweep_missing(&self) -> Vec<KeyCode> {
        self.expected_keys
            .iter()
            .filter(|k| !self.seen_keys.contains(k))
            .copied()
            .collect()
    }

    /// Start (or restart) the guided sequence.
    pub fn start(&mut self) {
        self.start_at(Instant::now());
    }

    fn start_at(&mut self, now: Instant) {
        self.reset_observations();
        self.steps = vec![StepKind::Idle, StepKind::Sweep];
        if self.input_source.has_release_events() {
            self.steps.push(StepKind::Hold);
            self.steps.push(StepKind::Rollover);
        } else {
            self.outcomes
                .push((StepKind::Hold, StepOutcome::Unsupported));
            self.outcomes
                .push((StepKind::Rollover, StepOutcome::Unsupported));
        }
        self.steps.push(StepKind::RapidTap);
        self.current = 0;
        self.state = AutoTestState::Running;
        self.started_at = Some(now);
        self.finished_at = None;
        self.step_started = Some(now);
    }

    /// Stop the sequence early. Observations so far are kept for analysis.
    pub fn abort(&mut self) {
        if self.state == AutoTestState::Running {
            if let Some(step) = self.current_step() {
                self.outcomes.push((step, StepOutcome::Aborted));
            }
            self.state = AutoTestState::Aborted;
            self.finished_at = Some(Instant::now());
        }
    }

    /// Skip the current step.
    pub fn skip_step(&mut self) {
        self.skip_step_at(Instant::now());
    }

    fn skip_step_at(&mut self, now: Instant) {
        if let Some(step) = self.current_step() {
            self.finish_step(step, StepOutcome::Skipped, now);
        }
    }

    /// Reset to the not-started state and clear all observations.
    pub fn reset(&mut self) {
        self.reset_observations();
        self.steps.clear();
        self.current = 0;
        self.state = AutoTestState::Idle;
        self.started_at = None;
        self.finished_at = None;
        self.step_started = None;
    }

    fn reset_observations(&mut self) {
        self.outcomes.clear();
        self.phantom_events.clear();
        self.idle_pressed.clear();
        self.seen_keys.clear();
        self.releases_seen = false;
        self.sweep_progress_at = None;
        self.press_times.clear();
        self.longest_hold = None;
        self.hold_pressed = false;
        self.rollover_pressed.clear();
        self.rollover_max = 0;
        self.rollover_peak_at = None;
        self.rapid_started = None;
        self.rapid_last = None;
        self.rapid_presses = 0;
        self.presses_during_run = 0;
    }

    /// Advance timers and step completion. Returns `true` when the whole
    /// sequence just finished on this call.
    pub fn tick(&mut self) -> bool {
        self.tick_at(Instant::now())
    }

    /// [`AutoTest::tick`] at a given time (deterministic tests, replays).
    pub fn tick_at(&mut self, now: Instant) -> bool {
        let Some(step) = self.current_step() else {
            return false;
        };
        let elapsed = now.saturating_duration_since(self.step_started.unwrap_or(now));

        let passed = match step {
            StepKind::Idle => elapsed >= IDLE_DURATION,
            StepKind::Sweep => self.sweep_missing().is_empty(),
            StepKind::Hold => self.longest_hold.is_some(),
            StepKind::Rollover => {
                // Done once the user has let go after reaching a good peak.
                // A lower peak keeps the step open (until it times out or is
                // skipped): the first attempt is often just a few keys.
                self.rollover_max >= GOOD_ROLLOVER
                    && self.rollover_pressed.is_empty()
                    && self
                        .rollover_peak_at
                        .is_some_and(|t| now.saturating_duration_since(t) >= Duration::from_secs(1))
            }
            StepKind::RapidTap => self
                .rapid_started
                .is_some_and(|t| now.saturating_duration_since(t) >= RAPID_TAP_DURATION),
        };

        if passed {
            // The idle step "passing" just means the time elapsed; whether it
            // found phantom input is judged in the analysis.
            self.finish_step(step, StepOutcome::Passed, now);
        } else if self.step_deadline(step).is_some_and(|d| now >= d) {
            self.finish_step(step, StepOutcome::TimedOut, now);
        } else {
            return false;
        }
        self.state == AutoTestState::Complete
    }

    /// When the current step times out. The sweep is extended while keys
    /// keep registering (see [`SWEEP_GRACE`]).
    fn step_deadline(&self, step: StepKind) -> Option<Instant> {
        let deadline = self.step_started? + step.timeout();
        Some(match self.sweep_progress_at {
            Some(at) if step == StepKind::Sweep => deadline.max(at + SWEEP_GRACE),
            _ => deadline,
        })
    }

    fn finish_step(&mut self, step: StepKind, outcome: StepOutcome, now: Instant) {
        self.outcomes.push((step, outcome));
        self.current += 1;
        self.step_started = Some(now);
        if self.current >= self.steps.len() {
            self.state = AutoTestState::Complete;
            self.finished_at = Some(now);
        }
    }

    /// Feed a key event. Only does something while a step is running.
    pub fn process_event(&mut self, event: &KeyEvent) {
        let Some(step) = self.current_step() else {
            return;
        };
        // Evdev events carry kernel timestamps (CLOCK_MONOTONIC, the clock
        // behind `Instant`). The evdev copy of the keystroke that started the
        // test is read only after the terminal delivered it, so it predates
        // the start and is not part of the run.
        if self.started_at.is_some_and(|start| event.timestamp < start) {
            return;
        }

        // (Switch chatter is judged by HoldReleaseTest, which sees the same
        // events and leaves out keys whose releases come from the driver.)
        match event.event_type {
            KeyEventType::Press => self.presses_during_run += 1,
            KeyEventType::Release => self.releases_seen = true,
        }

        // Likewise the evdev copy of the Ctrl+N that skipped the previous
        // step is read after the skip: it belongs to that step, not this one
        // (it would start the rapid-tap timer before the user taps).
        if self
            .step_started
            .is_some_and(|start| event.timestamp < start)
        {
            return;
        }

        match step {
            StepKind::Idle => {
                match event.event_type {
                    KeyEventType::Press => {
                        self.idle_pressed.insert(event.key);
                    }
                    // A release for a key that was not pressed during this
                    // step belongs to the keystroke that started the test.
                    KeyEventType::Release if !self.idle_pressed.contains(&event.key) => {
                        return;
                    }
                    KeyEventType::Release => {}
                }
                self.phantom_events.push((event.key, event.event_type));
            }
            StepKind::Sweep => {
                if event.event_type == KeyEventType::Press
                    && self.seen_keys.insert(event.key)
                    && self.expected_keys.contains(&event.key)
                {
                    self.sweep_progress_at = Some(event.timestamp);
                }
            }
            StepKind::Hold => match event.event_type {
                KeyEventType::Press => {
                    self.hold_pressed = true;
                    self.press_times.insert(event.key, event.timestamp);
                }
                KeyEventType::Release => {
                    if let Some(start) = self.press_times.remove(&event.key) {
                        let held = event.timestamp.saturating_duration_since(start);
                        if held >= MIN_HOLD && self.longest_hold.is_none_or(|(_, best)| held > best)
                        {
                            self.longest_hold = Some((event.key, held));
                        }
                    }
                }
            },
            StepKind::Rollover => {
                match event.event_type {
                    KeyEventType::Press => {
                        self.rollover_pressed.insert(event.key);
                    }
                    KeyEventType::Release => {
                        self.rollover_pressed.remove(&event.key);
                    }
                }
                let count = self.rollover_pressed.len();
                if count > self.rollover_max {
                    self.rollover_max = count;
                    self.rollover_peak_at = Some(event.timestamp);
                } else if count == self.rollover_max && count > 0 {
                    self.rollover_peak_at = Some(event.timestamp);
                }
            }
            StepKind::RapidTap => {
                if event.event_type == KeyEventType::Press {
                    if self.rapid_started.is_none() {
                        self.rapid_started = Some(event.timestamp);
                        // Timer restarts at the first tap
                        self.step_started = Some(event.timestamp);
                    }
                    self.rapid_presses += 1;
                    self.rapid_last = Some(event.timestamp);
                }
            }
        }
    }

    fn outcome_of(&self, step: StepKind) -> Option<StepOutcome> {
        self.outcomes
            .iter()
            .find(|(s, _)| *s == step)
            .map(|(_, o)| *o)
    }

    /// Whether the step collected observations (ran to an end or was aborted mid-way).
    fn step_was_run(&self, step: StepKind) -> bool {
        matches!(
            self.outcome_of(step),
            Some(StepOutcome::Passed | StepOutcome::TimedOut | StepOutcome::Aborted)
        )
    }

    // ------------------------------------------------------------------------
    // Analysis
    // ------------------------------------------------------------------------

    /// Analyse the test data and produce findings, most severe first.
    ///
    /// Findings from the guided steps are only included for steps that ran;
    /// everything else is derived from the continuous tests and can be
    /// requested at any time.
    pub fn analyze(&self, ctx: &AnalysisContext<'_>) -> Vec<Finding> {
        let mut findings = Vec::new();
        let releases = self.input_source.has_release_events();
        // Whether held modifier keys show up at all (not on the Windows console)
        let modifiers_reported = self.input_source.can_report(keymap::KEY_LSHIFT);

        // --- Connection ---------------------------------------------------
        findings.extend(disconnect_finding(
            ctx.keyboard_disconnects,
            ctx.disconnected_devices,
        ));

        // --- Input source limitations -------------------------------------
        if !releases {
            // A limitation of the setup, not of the keyboard.
            findings.push(Finding::new(
                Severity::Info,
                "Limited capture",
                "Terminal reports key presses only",
                "Releases are synthesized, so hold, rollover, stuck-key, bounce and \
                 chatter checks are skipped.",
                if cfg!(target_os = "linux") {
                    "Run with sudo (or add your user to the 'input' group) for raw evdev \
                     access, or use a terminal with the kitty keyboard protocol."
                } else {
                    "Use a terminal that supports the kitty keyboard protocol \
                     (kitty, foot, Ghostty, Alacritty 0.13+, or WezTerm with \
                     enable_kitty_keyboard = true)."
                },
            ));
        }

        // --- Idle step: phantom input -------------------------------------
        match self.outcome_of(StepKind::Idle) {
            Some(StepOutcome::Passed | StepOutcome::TimedOut) => {
                findings.extend(phantom_finding(&self.phantom_events));
            }
            Some(StepOutcome::Aborted) => {
                // Stopped before the step ran its course, usually with
                // Ctrl+C: the Ctrl (and with evdev the C) pressed for it is
                // read before the terminal delivers the chord. Keys still down
                // when the run stopped are that gesture; anything earlier is
                // noted, not judged.
                let down = keys_left_down(&self.phantom_events);
                let earlier: Vec<KeyCode> = self
                    .phantom_events
                    .iter()
                    .map(|(k, _)| *k)
                    .filter(|k| !down.contains(k))
                    .collect();
                if !earlier.is_empty() {
                    findings.push(Finding::new(
                        Severity::Info,
                        "Phantom input",
                        format!(
                            "{} event(s) before the idle check was stopped (not judged)",
                            earlier.len()
                        ),
                        format!(
                            "Keys: {}",
                            key_list(earlier.into_iter().collect::<HashSet<_>>().into_iter(), 8)
                        ),
                        "Run the auto test again with hands off the keyboard for the idle check.",
                    ));
                }
            }
            _ => {}
        }

        // --- Sweep step: dead keys ----------------------------------------
        match self.outcome_of(StepKind::Sweep) {
            Some(StepOutcome::TimedOut) => {
                // A key held down through the sweep (jammed, or down since
                // before the run) cannot send a new press, but it did
                // register: it is a stuck key or a long hold, not dead.
                let held_down: HashSet<KeyCode> = ctx
                    .stickiness
                    .flagged_keys()
                    .into_iter()
                    .map(|(k, _, _)| k)
                    .collect();
                let missing: Vec<KeyCode> = self
                    .sweep_missing()
                    .into_iter()
                    .filter(|k| !held_down.contains(k))
                    .collect();
                findings.extend(missing_key_findings(self.input_source, &missing));
            }
            Some(StepOutcome::Skipped | StepOutcome::Aborted) => {
                let missing = self.sweep_missing();
                if !missing.is_empty() {
                    findings.push(Finding::new(
                        Severity::Info,
                        "Not tested",
                        format!("{} key(s) were not pressed in the sweep", missing.len()),
                        format!("Keys: {}", key_list(missing.iter().copied(), 12)),
                        "Run the auto test again without skipping to verify every key.",
                    ));
                }
            }
            _ => {}
        }
        if self.outcome_of(StepKind::Sweep).is_some() {
            let unreportable = self.unreportable_keys();
            if !unreportable.is_empty() {
                findings.push(Finding::new(
                    Severity::Info,
                    "Not checked",
                    format!(
                        "{} key(s) cannot be checked via {}",
                        unreportable.len(),
                        self.input_source.name()
                    ),
                    format!("Keys: {}", key_list(unreportable.iter().copied(), 12)),
                    unreportable_advice(self.input_source),
                ));
            }
        }
        // --- Stuck keys (continuous) --------------------------------------
        // Only a key that is *still* down past the threshold is stuck; one
        // that was held long and then released is a normal long hold. The
        // hold and rollover steps ask the user to hold keys, so keys held
        // during them are not judged until the step is over. (A key merely
        // down right now, below the threshold, is someone holding it: a
        // sweep key held into the next step, the Ctrl of Ctrl+C.)
        if releases {
            let threshold = ctx.stickiness.threshold();
            // Longest first
            let stuck = ctx.stickiness.stuck_keys();
            let holding_on_purpose = matches!(
                self.current_step(),
                Some(StepKind::Hold | StepKind::Rollover)
            );
            if !stuck.is_empty() && !holding_on_purpose {
                let detail = stuck
                    .iter()
                    .take(6)
                    .map(|(k, held)| {
                        format!(
                            "{} ({:.1}s)",
                            keymap::get_key_info(*k).name,
                            held.as_secs_f64()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                findings.push(Finding::new(
                    Severity::Critical,
                    "Stuck key",
                    format!(
                        "{} key(s) held down for more than {} ms",
                        stuck.len(),
                        threshold.as_millis()
                    ),
                    detail,
                    "If you are not holding it, the key is stuck: remove the keycap and \
                     clean the switch. If it persists, the switch or controller is faulty.",
                ));
            }
            // Holds that ended in a release (a key that is down again now is
            // judged above, not listed twice)
            let mut long_holds: Vec<(KeyCode, Duration, u32)> = ctx
                .stickiness
                .long_holds()
                .into_iter()
                .filter(|(k, _, _)| !stuck.iter().any(|(s, _)| s == k))
                .collect();
            if !long_holds.is_empty() {
                long_holds.sort_by_key(|(k, longest, _)| (std::cmp::Reverse(*longest), k.0));
                let detail = long_holds
                    .iter()
                    .take(6)
                    .map(|(k, longest, n)| {
                        format!(
                            "{} ({}x, longest {:.1}s)",
                            keymap::get_key_info(*k).name,
                            n,
                            longest.as_secs_f64()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                findings.push(Finding::new(
                    Severity::Info,
                    "Long hold",
                    format!(
                        "{} key(s) were held past the stuck-key threshold, then released",
                        long_holds.len()
                    ),
                    detail,
                    "Normal if you were holding them. If a key stayed down after you let \
                     go, it is sticking: watch it in the Sticky view.",
                ));
            }
        }

        // --- Bounce / chatter (continuous) --------------------------------
        // Both need kernel timestamps: a terminal stamps events when the app
        // reads them, so a press and release read in one burst look like a
        // bounce (e.g. over SSH or while the app is busy drawing).
        if self.input_source.has_precise_timestamps() {
            findings.extend(bounce_finding(&ctx.hold_release.bouncy_keys()));
        } else {
            if self.step_was_run(StepKind::RapidTap) && self.rapid_presses > 0 {
                findings.push(Finding::new(
                    Severity::Info,
                    "Key chatter",
                    "Chatter and bounce cannot be measured with terminal input",
                    "Terminal timestamps are taken when the app reads the event, so a \
                     switch firing twice cannot be told from a fast tap.",
                    if cfg!(target_os = "linux") {
                        "Use the evdev input path (sudo or the 'input' group) to check for \
                         chatter and bounce."
                    } else {
                        ""
                    },
                ));
            }
        }

        // --- Hold step ----------------------------------------------------
        match self.outcome_of(StepKind::Hold) {
            Some(StepOutcome::Passed) => {
                if let Some((key, held)) = self.longest_hold {
                    findings.push(Finding::new(
                        Severity::Info,
                        "Hold/release",
                        "Key hold and release reported correctly",
                        format!(
                            "{} held for {:.2}s",
                            keymap::get_key_info(key).name,
                            held.as_secs_f64()
                        ),
                        "",
                    ));
                }
            }
            Some(StepOutcome::TimedOut) => findings.push(hold_timeout_finding(
                self.input_source,
                self.hold_pressed,
                self.releases_seen,
            )),
            _ => {}
        }

        // --- Rollover -----------------------------------------------------
        if let Some(finding) = rollover_finding(
            self.outcome_of(StepKind::Rollover),
            self.rollover_max,
            modifiers_reported,
        ) {
            findings.push(finding);
        }
        if ctx.rollover.ghost_count() > 0 {
            findings.push(Finding::new(
                Severity::Critical,
                "Ghosting",
                format!("{} ghost key event(s) detected", ctx.rollover.ghost_count()),
                "Keys registered that were not pressed while several others were held.",
                "This is a keyboard matrix limitation; avoid the affected combinations \
                 or use a keyboard with anti-ghosting.",
            ));
        } else if self.step_was_run(StepKind::Rollover) {
            // Nothing tells the rollover test which keys are really held, so
            // a ghost is indistinguishable from a press: say so, not "none".
            findings.push(Finding::new(
                Severity::Info,
                "Ghosting",
                "Not measured",
                "A ghost key arrives as an ordinary key press, so it cannot be told \
                 apart automatically.",
                "While holding several keys, watch the keyboard view: a key that lights \
                 up without being pressed is a ghost.",
            ));
        }

        // --- Polling / timing --------------------------------------------
        if self.input_source.has_precise_timestamps() {
            let samples = ctx.polling.quantization_samples();
            if samples >= super::polling::MIN_QUANTIZATION_SAMPLES {
                match ctx.polling.estimated_poll_interval_us() {
                    Some(step_us) => {
                        let hz = 1_000_000.0 / step_us as f64;
                        findings.push(Finding::new(
                            Severity::Info,
                            "Polling rate",
                            format!(
                                "Keyboard reports every {} ms (~{:.0} Hz)",
                                step_us / 1000,
                                hz
                            ),
                            format!(
                                "Inferred from the timing of {} gaps between key events.",
                                samples
                            ),
                            if hz < 200.0 {
                                "125 Hz is the USB default and fine for typing; gaming \
                                 keyboards usually offer 500-1000 Hz in their software."
                            } else {
                                ""
                            },
                        ));
                    }
                    // Same wording as the Polling view: Bluetooth keyboards
                    // show no grid either, so no rate is claimed
                    None => findings.push(Finding::new(
                        Severity::Info,
                        "Polling rate",
                        "No 125/250/500 Hz report grid visible (1000+ Hz, or Bluetooth)",
                        format!("{} gaps between key events analysed.", samples),
                        "",
                    )),
                }
            }
        } else if self.step_was_run(StepKind::RapidTap) && self.rapid_presses > 0 {
            findings.push(Finding::new(
                Severity::Info,
                "Polling rate",
                "Polling rate cannot be inferred from terminal input",
                "Terminal timestamps carry too much jitter for quantization analysis.",
                "Use the evdev input path (Linux) for a polling-rate estimate.",
            ));
        }
        if self.step_was_run(StepKind::RapidTap) && self.rapid_presses > 0 {
            let span = match (self.rapid_started, self.rapid_last) {
                (Some(first), Some(last)) => last.saturating_duration_since(first),
                _ => Duration::ZERO,
            };
            let title = match tap_rate(self.rapid_presses, span) {
                Some(rate) => format!(
                    "{} presses in {:.1}s ({:.1} per second)",
                    self.rapid_presses,
                    span.as_secs_f64(),
                    rate
                ),
                None => format!(
                    "{} press(es), too few to measure a rate",
                    self.rapid_presses
                ),
            };
            findings.push(Finding::new(
                Severity::Info,
                "Rapid tap",
                title,
                if self.input_source.has_precise_timestamps() {
                    ""
                } else if self.input_source.has_release_events() {
                    // Every tap is counted (each has its release), but each
                    // is stamped when it was read.
                    "Timed from when the terminal delivered the taps, so the rate is \
                     approximate."
                } else {
                    // Without releases the terminal listener tells taps from
                    // autorepeat by their timing alone: taps resuming faster
                    // than ~8 per second after a 0.2-2 s pause read as a held
                    // key's autorepeat (as fast as some repeat rates) and are
                    // not counted.
                    "Without key releases taps are told from a held key's autorepeat by \
                     their timing: fast taps after a pause can be missed, so the count \
                     and the rate are approximate."
                },
                "",
            ));
        }

        // --- Shortcuts / OEM / virtual ------------------------------------
        if ctx.shortcuts.conflict_count() > 0 {
            findings.push(Finding::new(
                Severity::Info,
                "Shortcuts",
                format!(
                    "{} known system shortcut combo(s) were pressed",
                    ctx.shortcuts.conflict_count()
                ),
                "The desktop or terminal may have intercepted them before this app.",
                "See the Shortcuts view for the combinations.",
            ));
        }
        let unknown = ctx.oem_keys.detected_unknown();
        if !unknown.is_empty() {
            let mut codes: Vec<(u16, u32)> = unknown.iter().map(|(c, n)| (*c, *n)).collect();
            codes.sort();
            let detail = codes
                .iter()
                .take(8)
                .map(|(c, n)| format!("0x{:03X} ({}x)", c, n))
                .collect::<Vec<_>>()
                .join(", ");
            findings.push(Finding::new(
                Severity::Info,
                "Unknown keys",
                format!("{} unrecognised scancode(s) seen", codes.len()),
                detail,
                "Usually vendor/OEM keys. Map them in the OEM/FN view ('a') or config.",
            ));
        }
        match ctx.virtual_detect.diagnostic() {
            DiagnosticResult::HardwareIssue => findings.push(Finding::new(
                Severity::Critical,
                "Hardware",
                "Virtual keys work but physical keys do not",
                "Software path is fine; the keyboard hardware or its connection is at fault.",
                "Try another USB port/cable or keyboard.",
            )),
            DiagnosticResult::SoftwareIssue => findings.push(Finding::new(
                Severity::Warning,
                "Software",
                "Neither physical nor virtual keys were received",
                "Points at a driver, permission or focus problem rather than the keyboard.",
                "Check input permissions and that this window has focus.",
            )),
            _ => {}
        }

        // --- Nothing wrong? -----------------------------------------------
        if !findings.iter().any(Finding::is_issue) {
            let scope = if self.state == AutoTestState::Complete {
                "Auto test complete: no issues detected"
            } else if self.presses_during_run > 0 || ctx.keyboard_state.total_events() > 0 {
                "No issues detected so far"
            } else {
                "No input yet"
            };
            findings.insert(
                0,
                Finding::new(
                    Severity::Info,
                    "Summary",
                    scope,
                    format!(
                        "{} events analysed via {}.",
                        ctx.keyboard_state.total_events(),
                        self.input_source.name()
                    ),
                    "",
                ),
            );
        }

        findings.sort_by_key(|f| std::cmp::Reverse(f.severity));
        findings
    }

    /// Render the auto-test view: status, current instruction and findings.
    pub fn results(&self, findings: &[Finding]) -> Vec<TestResult> {
        let mut rows = Vec::new();
        rows.push(TestResult::info("--- Auto Test ---", ""));

        match self.state {
            AutoTestState::Idle => {
                rows.push(TestResult::info(
                    "Status",
                    "Not started - press A to run the automatic diagnostic",
                ));
                rows.push(TestResult::info(
                    "What it does",
                    "Idle check, key sweep, hold, rollover, rapid tap, then a report",
                ));
                rows.push(TestResult::info("Input source", self.input_source.name()));
            }
            AutoTestState::Running => {
                let step = self.current_step().unwrap_or(StepKind::Idle);
                rows.push(TestResult::new(
                    "Status",
                    format!(
                        "Running step {}/{}: {}",
                        self.current + 1,
                        self.steps.len(),
                        step.title()
                    ),
                    ResultStatus::Ok,
                ));
                rows.push(TestResult::new(
                    "Do this",
                    step.instruction(),
                    ResultStatus::Warning,
                ));
                let remaining = self
                    .step_deadline(step)
                    .map(|d| d.saturating_duration_since(Instant::now()))
                    .unwrap_or_default()
                    .as_secs();
                rows.push(TestResult::info(
                    "Progress",
                    format!(
                        "{} {:>3.0}%  ({}s left in step)",
                        progress_bar(self.step_fraction(step), 20),
                        self.step_fraction(step) * 100.0,
                        remaining
                    ),
                ));
                match step {
                    StepKind::Idle => rows.push(TestResult::info(
                        "Phantom events",
                        format!("{}", self.phantom_events.len()),
                    )),
                    StepKind::Sweep => {
                        rows.push(TestResult::info(
                            "Keys verified",
                            format!("{} / {}", self.sweep_seen_count(), self.expected_keys.len()),
                        ));
                        let missing = self.sweep_missing();
                        if !missing.is_empty() {
                            rows.push(TestResult::info(
                                "Still missing",
                                key_list(missing.iter().copied(), 14),
                            ));
                            rows.push(TestResult::info(
                                "No such key?",
                                "Press Ctrl+N if your keyboard lacks a listed key",
                            ));
                        }
                        let unreportable = self.unreportable_keys();
                        if !unreportable.is_empty() {
                            rows.push(TestResult::info(
                                "Not checkable",
                                format!(
                                    "{} (this input source cannot report them)",
                                    key_list(unreportable.iter().copied(), 8)
                                ),
                            ));
                        }
                    }
                    StepKind::Hold => rows.push(TestResult::info(
                        "Longest hold",
                        match self.longest_hold {
                            Some((k, d)) => {
                                format!("{} {:.2}s", keymap::get_key_info(k).name, d.as_secs_f64())
                            }
                            None => format!("none yet (need {} ms)", MIN_HOLD.as_millis()),
                        },
                    )),
                    StepKind::Rollover => rows.push(TestResult::info(
                        "Keys held",
                        format!(
                            "{} now, peak {} (step ends at {}+ once all are released)",
                            self.rollover_pressed.len(),
                            self.rollover_max,
                            GOOD_ROLLOVER
                        ),
                    )),
                    StepKind::RapidTap => rows.push(TestResult::info(
                        "Presses",
                        if self.rapid_started.is_none() {
                            "waiting for first tap".to_string()
                        } else if self.input_source.has_release_events() {
                            format!("{}", self.rapid_presses)
                        } else {
                            // Taps told from autorepeat by timing alone
                            format!("{} (approximate: no key releases)", self.rapid_presses)
                        },
                    )),
                }
                rows.push(TestResult::info(
                    "Controls",
                    "Ctrl+N skip step  |  Ctrl+C abort",
                ));
            }
            AutoTestState::Complete | AutoTestState::Aborted => {
                let issues = findings.iter().filter(|f| f.is_issue()).count();
                let label = if self.state == AutoTestState::Complete {
                    "Complete"
                } else {
                    "Aborted"
                };
                let status = issues_status(findings);
                let duration = match (self.started_at, self.finished_at) {
                    (Some(s), Some(f)) => f.saturating_duration_since(s).as_secs(),
                    _ => 0,
                };
                rows.push(TestResult::new(
                    "Status",
                    format!(
                        "{} in {}s - {} issue(s) found. Press A to run again, e to export.",
                        label, duration, issues
                    ),
                    status,
                ));
                let steps = self
                    .outcomes
                    .iter()
                    .map(|(s, o)| {
                        format!(
                            "{} {}",
                            s.title(),
                            match o {
                                StepOutcome::Passed => "✓",
                                StepOutcome::TimedOut => "⏱",
                                StepOutcome::Skipped => "skipped",
                                StepOutcome::Unsupported => "n/a",
                                StepOutcome::Aborted => "aborted",
                            }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                rows.push(TestResult::info("Steps", steps));
            }
        }

        rows.push(TestResult::info("", ""));
        rows.push(TestResult::info("--- Findings ---", ""));
        if findings.is_empty() {
            rows.push(TestResult::info("Findings", "none yet"));
        }
        for finding in findings {
            rows.extend(finding.to_results());
        }
        rows
    }
}

/// Keys the sweep expects for a layout: the main block plus the arrow keys.
pub fn expected_keys_for_layout(layout: KeyboardLayout) -> Vec<KeyCode> {
    let mut keys: Vec<KeyCode> = layout_rows(layout)
        .iter()
        .flat_map(|row| row.iter().map(|k| KeyCode(k.code)))
        .collect();
    keys.extend([KeyCode(103), KeyCode(105), KeyCode(108), KeyCode(106)]);
    let mut seen = HashSet::new();
    keys.retain(|k| seen.insert(*k));
    keys
}

/// Findings for keys that never registered in a sweep that timed out.
///
/// Only evdev sees every key the keyboard sends, so only there is a missing
/// key evidence of a dead switch. Even then some keys are not on every
/// keyboard. Through a terminal a key may also just not be passed on.
fn missing_key_findings(source: InputSource, missing: &[KeyCode]) -> Vec<Finding> {
    let mut findings = Vec::new();
    let keys = |list: &[KeyCode]| format!("Keys: {}", key_list(list.iter().copied(), 12));
    if source == InputSource::Evdev {
        let (optional, required): (Vec<KeyCode>, Vec<KeyCode>) = missing
            .iter()
            .partition(|k| KEYS_NOT_ON_EVERY_KEYBOARD.contains(k));
        if !required.is_empty() {
            findings.push(Finding::new(
                Severity::Critical,
                "Dead keys",
                format!("{} key(s) never registered", required.len()),
                keys(&required),
                "If you did press them, the switch or its matrix trace is faulty. \
                 Clean or replace the switch; on laptops check the ribbon cable.",
            ));
        }
        if !optional.is_empty() {
            findings.push(Finding::new(
                Severity::Warning,
                "Not registered",
                format!(
                    "{} key(s) that not every keyboard has never registered",
                    optional.len()
                ),
                keys(&optional),
                "Many laptops have no Right Ctrl, gaming keyboards switch the Windows key \
                 off in game mode, and ISO/JIS-only keys depend on the physical layout. \
                 If your keyboard has them and you pressed them, the switch is faulty.",
            ));
        }
        return findings;
    }

    // Letters and the named keys reach the terminal on any Latin layout;
    // digits and punctuation type other characters on non-US layouts, and
    // the desktop may keep modifier keys to itself.
    let (definite, uncertain): (Vec<KeyCode>, Vec<KeyCode>) =
        missing.iter().partition(|k| reported_on_any_layout(**k));
    let better = if cfg!(target_os = "linux") {
        " Run with sudo (or join the 'input' group) so evdev can tell for certain."
    } else {
        ""
    };
    if !definite.is_empty() {
        findings.push(Finding::new(
            Severity::Warning,
            "Not registered",
            format!(
                "{} key(s) never registered through the terminal",
                definite.len()
            ),
            keys(&definite),
            format!(
                "If you pressed them, the keys may be dead (unless a non-Latin layout such \
                 as Cyrillic was active); check them in a text editor.{}",
                better
            ),
        ));
    }
    if !uncertain.is_empty() {
        findings.push(Finding::new(
            Severity::Info,
            "Not registered",
            format!(
                "{} key(s) never registered through the terminal",
                uncertain.len()
            ),
            keys(&uncertain),
            format!(
                "Not evidence of a fault: on non-US layouts these keys type characters \
                 the terminal cannot map back, the desktop may keep modifier keys to \
                 itself, or they were not pressed.{}",
                better
            ),
        ));
    }
    findings
}

/// Keys a terminal reports the same way on any Latin layout: the letters
/// (permuted on QWERTZ/AZERTY, but all present) and the named keys.
fn reported_on_any_layout(key: KeyCode) -> bool {
    matches!(
        key.0,
        1 | 14 | 15 | 28 | 57 | 16..=25 | 30..=38 | 44..=50 | 103 | 105 | 106 | 108
    )
}

/// What to do about keys the input source cannot report.
fn unreportable_advice(source: InputSource) -> &'static str {
    if cfg!(target_os = "linux") {
        "Run with sudo (or add your user to the 'input' group) so evdev can check them."
    } else if source == InputSource::TerminalPressOnly {
        "A terminal with the kitty keyboard protocol (kitty, Ghostty, Alacritty 0.13+, \
         or WezTerm with enable_kitty_keyboard = true) reports modifier keys."
    } else {
        "This console does not report them; check them in another application."
    }
}

/// The finding for a keyboard that dropped off `disconnects` times.
///
/// A keyboard that disappears mid-session (flaky cable or port, weak
/// wireless link) is a fault in its own right, and it explains keys that
/// "never registered" meanwhile. An unplug made on purpose, or a wireless
/// keyboard going to sleep, looks exactly the same, so it is a conditionally
/// worded Warning rather than a verdict.
fn disconnect_finding(disconnects: u32, devices: &[String]) -> Option<Finding> {
    (disconnects > 0).then(|| {
        let names = if devices.is_empty() {
            String::new()
        } else {
            format!("{}. ", devices.join(", "))
        };
        Finding::new(
            Severity::Warning,
            "Connection",
            format!("Keyboard disconnected {} time(s)", disconnects),
            format!(
                "{}Keys pressed while it was gone were not seen by any test. A wireless \
                 keyboard that went to sleep while idle shows up here too.",
                names
            ),
            "If you did not unplug it or switch it off, check the cable and USB port \
             (try another port), or the wireless link and batteries.",
        )
    })
}

/// Keys whose last event in `events` is a press: still down at the end.
fn keys_left_down(events: &[(KeyCode, KeyEventType)]) -> HashSet<KeyCode> {
    let mut down = HashSet::new();
    for (key, kind) in events {
        match kind {
            KeyEventType::Press => down.insert(*key),
            KeyEventType::Release => down.remove(key),
        };
    }
    down
}

/// The finding for input during a completed idle step, if there was any.
///
/// Whole keystrokes, each key pressed and released at most twice, are what
/// a person does (pressing the start key again, a finger brushing a key),
/// so they are a Warning worded as a condition. A key left down or firing
/// over and over is Critical, still worded for the user who did touch it.
fn phantom_finding(events: &[(KeyCode, KeyEventType)]) -> Option<Finding> {
    if events.is_empty() {
        return None;
    }
    let keys: HashSet<KeyCode> = events.iter().map(|(k, _)| *k).collect();
    let presses = |key: KeyCode| {
        events
            .iter()
            .filter(|(k, kind)| *k == key && *kind == KeyEventType::Press)
            .count()
    };
    let keystrokes = keys_left_down(events).is_empty() && keys.iter().all(|k| presses(*k) <= 2);
    let detail = format!("Keys: {}", key_list(keys.into_iter(), 8));
    Some(if keystrokes {
        Finding::new(
            Severity::Warning,
            "Phantom input",
            format!("{} event(s) arrived during the idle check", events.len()),
            detail,
            "If you did not touch the keyboard, a key fired on its own (debris, liquid, a \
             matrix fault) or software injected it. If you pressed a key, this is not a \
             fault: run the test again with hands off.",
        )
    } else {
        Finding::new(
            Severity::Critical,
            "Phantom input",
            format!(
                "{} event(s) arrived while nothing was pressed",
                events.len()
            ),
            detail,
            "If you did not touch the keyboard, a key is stuck down, chattering, or something \
             else is injecting input: check for debris under the listed keys and for \
             macro/remap software.",
        )
    })
}

/// The finding for keys that bounced (see [`HoldReleaseTest::bouncy_keys`]).
///
/// One doubled keystroke may be a worn switch, but also a wireless keyboard
/// whose delayed report reached the computer together with the next one,
/// which kernel timestamps cannot tell apart. A key that bounces again is a
/// fault; a single bounce is a Warning worded as a condition.
fn bounce_finding(bouncy: &[(KeyCode, u32)]) -> Option<Finding> {
    if bouncy.is_empty() {
        return None;
    }
    let mut bouncy = bouncy.to_vec();
    bouncy.sort_by_key(|(k, n)| (std::cmp::Reverse(*n), k.0));
    let total: u32 = bouncy.iter().map(|(_, n)| n).sum();
    let detail = bouncy
        .iter()
        .take(6)
        .map(|(k, n)| {
            let plural = if *n == 1 { "" } else { "s" };
            format!("{} ({} bounce{})", keymap::get_key_info(*k).name, n, plural)
        })
        .collect::<Vec<_>>()
        .join(", ");
    Some(if bouncy.iter().any(|(_, n)| *n >= 2) {
        Finding::new(
            Severity::Critical,
            "Switch bounce",
            format!("{} bounce(s) on {} key(s)", total, bouncy.len()),
            detail,
            "Contact bounce causes doubled characters. Clean the switch or increase \
             firmware debounce; replace the switch if it continues. (A wireless keyboard \
             can look the same when radio reports are delayed: retest it close to the \
             receiver or with the cable.)",
        )
    } else {
        Finding::new(
            Severity::Warning,
            "Switch bounce",
            format!("{} key(s) registered one press twice", bouncy.len()),
            detail,
            "A worn switch, or a wireless keyboard delivering a delayed report together \
             with the next one. Press the key several times: if it bounces again, clean or \
             replace the switch.",
        )
    })
}

/// The finding for a hold step that timed out.
///
/// With no key pressed in it the step was simply not done, and with key
/// releases arriving elsewhere in the run release reporting evidently works:
/// both are information. Only presses without a single release anywhere
/// point at releases not reaching the app.
fn hold_timeout_finding(source: InputSource, pressed: bool, releases_seen: bool) -> Finding {
    let expected = format!(
        "Expected a single key held for at least {} ms followed by its release.",
        MIN_HOLD.as_millis()
    );
    let console = if source.can_report(keymap::KEY_LSHIFT) {
        ""
    } else {
        " This console does not report Shift, Ctrl, Alt or the Windows key: hold a letter \
         or Space."
    };
    if !pressed {
        Finding::new(
            Severity::Info,
            "Hold/release",
            "Hold step produced no key presses",
            expected,
            format!(
                "Run the auto test again to check hold and release.{}",
                console
            ),
        )
    } else if releases_seen {
        Finding::new(
            Severity::Info,
            "Hold/release",
            "Hold step not completed",
            format!(
                "{} Key releases did arrive during the run, so they are reported.",
                expected
            ),
            format!(
                "Run the auto test again and hold one key for about a second.{}",
                console
            ),
        )
    } else {
        Finding::new(
            Severity::Warning,
            "Hold/release",
            "No key release was observed",
            expected,
            if source == InputSource::Evdev {
                "If you held and released a key, its release did not reach the app: check \
                 for software that grabs the keyboard (remappers, macro tools)."
                    .to_string()
            } else {
                format!(
                    "If you held and released a key, its release did not reach the app: \
                     check for remapping software{}.{}",
                    if cfg!(target_os = "linux") {
                        ", or try the evdev input path"
                    } else {
                        ""
                    },
                    console
                )
            },
        )
    }
}

/// How to colour a summary of `findings`: OK without issues, otherwise the
/// colour of the most severe issue (a red verdict only for a Critical one).
pub fn issues_status(findings: &[Finding]) -> ResultStatus {
    findings
        .iter()
        .filter(|f| f.is_issue())
        .map(|f| f.severity)
        .max()
        .map_or(ResultStatus::Ok, Severity::to_status)
}

/// Judge the rollover step from how it ended and its peak.
///
/// The step only passes at [`GOOD_ROLLOVER`] keys. A lower peak at the timeout
/// may be the keyboard's limit or just fewer keys pressed, which the app
/// cannot tell apart, so it is worded as a condition, never as a verdict.
/// Without `modifiers_reported` held modifier keys were not counted.
fn rollover_finding(
    outcome: Option<StepOutcome>,
    max: usize,
    modifiers_reported: bool,
) -> Option<Finding> {
    let peak = format!(
        "Peak of {} simultaneous keys during the rollover step.",
        max
    );
    match outcome {
        Some(StepOutcome::Passed) | Some(StepOutcome::TimedOut) if max >= GOOD_ROLLOVER => {
            Some(Finding::new(
                Severity::Info,
                "Rollover",
                format!("Rollover OK: {} keys at once", max),
                peak,
                "",
            ))
        }
        Some(StepOutcome::TimedOut) if max == 0 => Some(Finding::new(
            Severity::Info,
            "Rollover",
            "Rollover step produced no key presses",
            "",
            "Run the auto test again to measure rollover.",
        )),
        Some(StepOutcome::TimedOut) => Some(Finding::new(
            Severity::Warning,
            "Rollover",
            format!("No more than {} key(s) registered at once", max),
            if modifiers_reported {
                peak
            } else {
                format!(
                    "{} Shift, Ctrl, Alt and the Windows key are not reported by this \
                     console and were not counted.",
                    peak
                )
            },
            format!(
                "If you held more keys than that, the keyboard limits rollover, which \
                 blocks key combinations in games and shortcuts (some keyboards only \
                 reach full rollover over USB). If you held fewer, this is not a fault: \
                 run again and hold {} or more keys.",
                GOOD_ROLLOVER
            ),
        )),
        Some(StepOutcome::Skipped | StepOutcome::Aborted) if max > 0 => Some(Finding::new(
            Severity::Info,
            "Rollover",
            format!("Peak of {} key(s) at once (step not finished)", max),
            "Rollover is only judged when the step runs to the end.",
            "",
        )),
        _ => None,
    }
}

/// Comma-separated key names, truncated with a count of the remainder.
fn key_list(keys: impl Iterator<Item = KeyCode>, max: usize) -> String {
    let mut names: Vec<String> = keys
        .map(|k| keymap::get_key_info(k).name.to_string())
        .collect();
    names.sort();
    let total = names.len();
    let mut out = names.into_iter().take(max).collect::<Vec<_>>().join(", ");
    if total > max {
        out.push_str(&format!(" … +{} more", total - max));
    }
    out
}

/// A simple text progress bar.
fn progress_bar(fraction: f64, width: usize) -> String {
    let filled = ((fraction.clamp(0.0, 1.0) * width as f64).round()) as usize;
    format!("[{}{}]", "#".repeat(filled), "-".repeat(width - filled))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::test_helpers::{make_event, press, release};
    use crate::tests::KeyboardTest;

    fn ctx_parts() -> (
        PollingRateTest,
        HoldReleaseTest,
        StickinessTest,
        RolloverTest,
        EventTimingTest,
        ShortcutTest,
        VirtualKeyboardTest,
        OemKeyTest,
        KeyboardState,
    ) {
        (
            PollingRateTest::new(10, 100),
            HoldReleaseTest::new(5),
            StickinessTest::new(2000),
            RolloverTest::new(),
            EventTimingTest::new(),
            ShortcutTest::new(),
            VirtualKeyboardTest::new(),
            OemKeyTest::new(),
            KeyboardState::new(),
        )
    }

    macro_rules! ctx {
        ($p:expr) => {
            ctx!($p, 0, &[])
        };
        ($p:expr, $disconnects:expr, $devices:expr) => {
            AnalysisContext {
                polling: &$p.0,
                hold_release: &$p.1,
                stickiness: &$p.2,
                rollover: &$p.3,
                event_timing: &$p.4,
                shortcuts: &$p.5,
                virtual_detect: &$p.6,
                oem_keys: &$p.7,
                keyboard_state: &$p.8,
                keyboard_disconnects: $disconnects,
                disconnected_devices: $devices,
            }
        };
    }

    fn evt(key: u16, kind: KeyEventType, at: Instant, delta_us: u64) -> KeyEvent {
        make_event(KeyCode(key), kind, at, delta_us)
    }

    #[test]
    fn new_test_is_idle() {
        let t = AutoTest::new(InputSource::Evdev);
        assert_eq!(t.state(), AutoTestState::Idle);
        assert!(!t.is_running());
        assert_eq!(t.progress(), 0.0);
        assert!(t.current_step().is_none());
        assert!(!t.expected_keys().is_empty());
    }

    #[test]
    fn start_builds_full_sequence_with_releases() {
        let mut t = AutoTest::new(InputSource::Evdev);
        t.start();
        assert!(t.is_running());
        assert_eq!(
            t.steps(),
            &[
                StepKind::Idle,
                StepKind::Sweep,
                StepKind::Hold,
                StepKind::Rollover,
                StepKind::RapidTap
            ]
        );
        assert_eq!(t.current_step(), Some(StepKind::Idle));
    }

    #[test]
    fn start_skips_release_dependent_steps_without_releases() {
        let mut t = AutoTest::new(InputSource::TerminalPressOnly);
        t.start();
        assert_eq!(
            t.steps(),
            &[StepKind::Idle, StepKind::Sweep, StepKind::RapidTap]
        );
        assert!(t
            .outcomes()
            .contains(&(StepKind::Hold, StepOutcome::Unsupported)));
    }

    #[test]
    fn idle_step_completes_after_duration_and_records_phantoms() {
        let mut t = AutoTest::new(InputSource::Evdev);
        let t0 = Instant::now();
        t.start_at(t0);
        assert!(!t.tick_at(t0 + Duration::from_secs(1)));
        assert_eq!(t.current_step(), Some(StepKind::Idle));
        t.process_event(&press(KeyCode(30)));
        assert!(!t.tick_at(t0 + IDLE_DURATION));
        assert_eq!(t.current_step(), Some(StepKind::Sweep));
        assert_eq!(t.outcomes()[0], (StepKind::Idle, StepOutcome::Passed));

        let parts = ctx_parts();
        let findings = t.analyze(&ctx!(parts));
        let phantom = findings
            .iter()
            .find(|f| f.category == "Phantom input")
            .expect("phantom finding");
        assert_eq!(phantom.severity, Severity::Critical);
        assert!(phantom.detail.contains('A'));
    }

    #[test]
    fn idle_step_ignores_release_of_the_starting_keystroke() {
        let mut t = AutoTest::new(InputSource::Evdev);
        let t0 = Instant::now();
        t.start_at(t0);
        // Shift and A were pressed *before* the test started; their releases
        // arrive during the idle step and must not count as phantom input.
        t.process_event(&release(KeyCode(42)));
        t.process_event(&release(KeyCode(30)));
        t.tick_at(t0 + IDLE_DURATION);
        let parts = ctx_parts();
        let findings = t.analyze(&ctx!(parts));
        assert!(!findings.iter().any(|f| f.category == "Phantom input"));

        // But a press followed by its release during idle is phantom input
        let mut t = AutoTest::new(InputSource::Evdev);
        t.start_at(t0);
        t.process_event(&press(KeyCode(30)));
        t.process_event(&release(KeyCode(30)));
        t.tick_at(t0 + IDLE_DURATION);
        let findings = t.analyze(&ctx!(parts));
        let f = findings
            .iter()
            .find(|f| f.category == "Phantom input")
            .expect("phantom");
        assert!(f.title.starts_with("2 event(s)"));
    }

    /// Findings of a run whose idle step saw `events`, then ran its course.
    fn idle_findings(events: &[(u16, KeyEventType)]) -> Vec<Finding> {
        let mut t = AutoTest::new(InputSource::Evdev);
        let t0 = Instant::now();
        t.start_at(t0);
        for (i, (key, kind)) in events.iter().enumerate() {
            let at = t0 + Duration::from_millis(500 + 60 * i as u64);
            t.process_event(&evt(*key, *kind, at, 0));
        }
        t.tick_at(t0 + IDLE_DURATION);
        let parts = ctx_parts();
        t.analyze(&ctx!(parts))
    }

    #[test]
    fn whole_keystrokes_during_idle_are_a_conditional_warning() {
        use KeyEventType::{Press as P, Release as R};
        // Shift+A pressed again right after starting: a person, most likely
        for events in [
            vec![(42, P), (30, P), (30, R), (42, R)],
            vec![(30, P), (30, R), (30, P), (30, R)],
        ] {
            let findings = idle_findings(&events);
            let f = findings
                .iter()
                .find(|f| f.category == "Phantom input")
                .unwrap();
            assert_eq!(f.severity, Severity::Warning, "{:?}", events);
            assert!(f.advice.starts_with("If you did not touch the keyboard"));
            assert!(f
                .advice
                .contains("If you pressed a key, this is not a fault"));
        }
        // A key left down, or one firing again and again, is a fault
        for events in [
            vec![(30, P)],
            vec![(30, P), (30, R), (30, P), (30, R), (30, P), (30, R)],
        ] {
            let findings = idle_findings(&events);
            let f = findings
                .iter()
                .find(|f| f.category == "Phantom input")
                .unwrap();
            assert_eq!(f.severity, Severity::Critical, "{:?}", events);
            assert!(f.advice.starts_with("If you did not touch the keyboard"));
        }
        assert!(idle_findings(&[])
            .iter()
            .all(|f| f.category != "Phantom input"));
    }

    #[test]
    fn stopping_the_idle_check_with_ctrl_c_is_not_phantom_input() {
        // Ctrl goes down (read from evdev, or reported by the kitty
        // protocol) before the terminal delivers Ctrl+C, which aborts
        let mut t = AutoTest::new(InputSource::Evdev);
        let t0 = Instant::now();
        t.start_at(t0);
        let ms = Duration::from_millis;
        t.process_event(&evt(29, KeyEventType::Press, t0 + ms(1200), 0));
        t.process_event(&evt(46, KeyEventType::Press, t0 + ms(1350), 0));
        t.abort();
        let parts = ctx_parts();
        let findings = t.analyze(&ctx!(parts));
        assert!(
            !findings.iter().any(|f| f.category == "Phantom input"),
            "{:?}",
            findings
        );
        assert!(!findings.iter().any(Finding::is_issue), "{:?}", findings);

        // A key that came and went before the abort is noted, not judged
        let mut t = AutoTest::new(InputSource::Evdev);
        t.start_at(t0);
        t.process_event(&evt(30, KeyEventType::Press, t0 + ms(400), 0));
        t.process_event(&evt(30, KeyEventType::Release, t0 + ms(470), 0));
        t.process_event(&evt(29, KeyEventType::Press, t0 + ms(1200), 0));
        t.abort();
        let findings = t.analyze(&ctx!(parts));
        let f = findings
            .iter()
            .find(|f| f.category == "Phantom input")
            .unwrap();
        assert_eq!(f.severity, Severity::Info);
        assert_eq!(f.detail, "Keys: A");
    }

    #[test]
    fn sweep_completes_when_all_expected_keys_seen() {
        let mut t = AutoTest::new(InputSource::Evdev);
        t.set_expected_keys(vec![KeyCode(30), KeyCode(31)]);
        let t0 = Instant::now();
        t.start_at(t0);
        t.tick_at(t0 + IDLE_DURATION); // -> Sweep
        assert_eq!(t.current_step(), Some(StepKind::Sweep));
        let s = t0 + IDLE_DURATION + Duration::from_millis(200);
        t.process_event(&evt(30, KeyEventType::Press, s, 0));
        t.process_event(&evt(30, KeyEventType::Release, s, 0));
        assert!(!t.tick_at(t0 + Duration::from_secs(4)));
        assert_eq!(t.sweep_seen_count(), 1);
        assert_eq!(t.sweep_missing(), vec![KeyCode(31)]);
        let s = t0 + Duration::from_secs(4);
        t.process_event(&evt(31, KeyEventType::Press, s, 0));
        t.process_event(&evt(31, KeyEventType::Release, s, 0));
        assert!(!t.tick_at(t0 + Duration::from_secs(5)));
        assert_eq!(t.current_step(), Some(StepKind::Hold));
    }

    #[test]
    fn sweep_timeout_reports_dead_keys() {
        let mut t = AutoTest::new(InputSource::Evdev);
        t.set_expected_keys(vec![KeyCode(30), KeyCode(31), KeyCode(32)]);
        let t0 = Instant::now();
        t.start_at(t0);
        t.tick_at(t0 + IDLE_DURATION);
        let s = t0 + Duration::from_secs(4);
        t.process_event(&evt(30, KeyEventType::Press, s, 0));
        t.process_event(&evt(30, KeyEventType::Release, s, 0));
        // Time out the sweep
        t.tick_at(t0 + IDLE_DURATION + StepKind::Sweep.timeout());
        assert_eq!(t.current_step(), Some(StepKind::Hold));

        let parts = ctx_parts();
        let findings = t.analyze(&ctx!(parts));
        let dead = findings
            .iter()
            .find(|f| f.category == "Dead keys")
            .expect("dead keys finding");
        assert_eq!(dead.severity, Severity::Critical);
        assert!(dead.title.starts_with("2 key(s)"));
        assert!(dead.detail.contains('S') && dead.detail.contains('D'));
    }

    #[test]
    fn key_held_down_through_the_sweep_is_stuck_not_dead() {
        // Backspace jammed since before the run never sends a press in the
        // sweep. It is reported as stuck (and later as a long hold if it
        // frees up), not as a dead key with "replace the switch" advice.
        let mut parts = ctx_parts();
        let jammed = Instant::now() - Duration::from_secs(5);
        parts
            .2
            .process_event(&evt(14, KeyEventType::Press, jammed, 0));
        parts.2.check_stuck_keys();
        let mut t = AutoTest::new(InputSource::Evdev);
        t.set_expected_keys(vec![KeyCode(14), KeyCode(30), KeyCode(31)]);
        let t0 = Instant::now();
        t.start_at(t0);
        t.tick_at(t0 + IDLE_DURATION);
        t.process_event(&evt(
            30,
            KeyEventType::Press,
            t0 + Duration::from_secs(4),
            0,
        ));
        t.tick_at(t0 + IDLE_DURATION + StepKind::Sweep.timeout());
        // (Held keys are not judged during the hold step that follows)
        t.abort();
        let findings = t.analyze(&ctx!(parts));
        let dead = findings.iter().find(|f| f.category == "Dead keys").unwrap();
        assert_eq!(dead.detail, "Keys: S");
        assert!(findings
            .iter()
            .any(|f| f.category == "Stuck key" && f.detail.starts_with("Backspace")));

        parts
            .2
            .process_event(&evt(14, KeyEventType::Release, Instant::now(), 0));
        let findings = t.analyze(&ctx!(parts));
        let dead = findings.iter().find(|f| f.category == "Dead keys").unwrap();
        assert_eq!(dead.detail, "Keys: S");
        assert!(findings.iter().any(|f| f.category == "Long hold"));
    }

    #[test]
    fn slow_but_steady_sweep_is_not_cut_off() {
        let mut t = AutoTest::new(InputSource::Evdev);
        t.set_expected_keys(vec![KeyCode(30), KeyCode(31), KeyCode(32)]);
        let t0 = Instant::now();
        t.start_at(t0);
        t.tick_at(t0 + IDLE_DURATION); // -> Sweep
        let ss = t0 + IDLE_DURATION;
        let secs = Duration::from_secs;
        t.process_event(&evt(30, KeyEventType::Press, ss + secs(60), 0));
        t.process_event(&evt(31, KeyEventType::Press, ss + secs(110), 0));
        // Past the 120 s timeout, but the user is still finding keys
        assert!(!t.tick_at(ss + secs(130)));
        assert_eq!(t.current_step(), Some(StepKind::Sweep));
        t.process_event(&evt(32, KeyEventType::Press, ss + secs(138), 0));
        t.tick_at(ss + secs(139));
        assert_eq!(t.outcome_of(StepKind::Sweep), Some(StepOutcome::Passed));
        let parts = ctx_parts();
        let findings = t.analyze(&ctx!(parts));
        assert!(!findings.iter().any(|f| f.category == "Dead keys"));

        // A sweep that stops making progress still times out: pressing a key
        // that was already verified does not extend it
        let mut t = AutoTest::new(InputSource::Evdev);
        t.set_expected_keys(vec![KeyCode(30), KeyCode(31), KeyCode(32)]);
        t.start_at(t0);
        t.tick_at(ss);
        t.process_event(&evt(30, KeyEventType::Press, ss + secs(110), 0));
        t.process_event(&evt(30, KeyEventType::Press, ss + secs(125), 0));
        assert!(!t.tick_at(ss + secs(139)));
        t.tick_at(ss + secs(140));
        assert_eq!(t.outcome_of(StepKind::Sweep), Some(StepOutcome::TimedOut));
        let findings = t.analyze(&ctx!(parts));
        let dead = findings.iter().find(|f| f.category == "Dead keys").unwrap();
        assert!(dead.title.starts_with("2 key(s)"), "{}", dead.title);
    }

    #[test]
    fn sweep_skip_reports_untested_keys_as_info() {
        let mut t = AutoTest::new(InputSource::Evdev);
        t.set_expected_keys(vec![KeyCode(30), KeyCode(31)]);
        let t0 = Instant::now();
        t.start_at(t0);
        t.tick_at(t0 + IDLE_DURATION);
        t.skip_step_at(t0 + Duration::from_secs(4));
        assert_eq!(t.current_step(), Some(StepKind::Hold));
        let parts = ctx_parts();
        let findings = t.analyze(&ctx!(parts));
        let f = findings
            .iter()
            .find(|f| f.category == "Not tested")
            .expect("not tested finding");
        assert_eq!(f.severity, Severity::Info);
    }

    #[test]
    fn hold_step_requires_minimum_hold() {
        let mut t = AutoTest::new(InputSource::Evdev);
        t.set_expected_keys(vec![]);
        let t0 = Instant::now();
        t.start_at(t0);
        t.tick_at(t0 + IDLE_DURATION); // -> Sweep (empty -> passes immediately)
        t.tick_at(t0 + IDLE_DURATION); // -> Hold
        assert_eq!(t.current_step(), Some(StepKind::Hold));

        let s = t0 + Duration::from_secs(4);
        t.process_event(&evt(30, KeyEventType::Press, s, 0));
        t.process_event(&evt(
            30,
            KeyEventType::Release,
            s + Duration::from_millis(200),
            0,
        ));
        assert!(!t.tick_at(s + Duration::from_millis(300)));
        assert_eq!(t.current_step(), Some(StepKind::Hold));

        t.process_event(&evt(30, KeyEventType::Press, s + Duration::from_secs(1), 0));
        t.process_event(&evt(
            30,
            KeyEventType::Release,
            s + Duration::from_secs(1) + MIN_HOLD,
            0,
        ));
        assert!(!t.tick_at(s + Duration::from_secs(2)));
        assert_eq!(t.current_step(), Some(StepKind::Rollover));
    }

    /// The Hold/release finding of a run whose hold step saw `events` (ms
    /// after the step started, key, kind) and then timed out.
    fn hold_timeout(source: InputSource, events: &[(u64, u16, KeyEventType)]) -> Finding {
        let (mut t, t0) = run_at_step(source, StepKind::Hold);
        for (ms, key, kind) in events {
            t.process_event(&evt(*key, *kind, t0 + Duration::from_millis(*ms), 0));
        }
        t.tick_at(t0 + StepKind::Hold.timeout());
        assert_eq!(t.outcome_of(StepKind::Hold), Some(StepOutcome::TimedOut));
        let parts = ctx_parts();
        let findings = t.analyze(&ctx!(parts));
        findings
            .into_iter()
            .find(|f| f.category == "Hold/release")
            .unwrap()
    }

    #[test]
    fn hold_step_timeout_blames_releases_only_when_none_arrived() {
        use KeyEventType::{Press as P, Release as R};
        // Releases did arrive (here: holds of 0.6 s, too short): the step was
        // just not completed, which is not a keyboard issue
        let f = hold_timeout(InputSource::Evdev, &[(1000, 30, P), (1600, 30, R)]);
        assert_eq!(f.severity, Severity::Info, "{:?}", f);
        assert!(!f.advice.contains("evdev"), "{}", f.advice);
        // Nothing pressed at all: the step was not done
        let f = hold_timeout(InputSource::Evdev, &[]);
        assert_eq!(f.severity, Severity::Info);
        assert!(f.title.contains("no key presses"), "{}", f.title);
        // Presses without a single release anywhere: releases do not arrive
        let f = hold_timeout(InputSource::Evdev, &[(1000, 30, P), (5000, 31, P)]);
        assert_eq!(f.severity, Severity::Warning);
        assert!(f.advice.starts_with("If you held and released a key"));
        // On evdev it does not suggest the input path already in use
        assert!(!f.advice.contains("evdev"), "{}", f.advice);
        let f = hold_timeout(InputSource::TerminalWithRelease, &[(1000, 30, P)]);
        assert_eq!(f.severity, Severity::Warning);
        assert_eq!(
            f.advice.contains("evdev"),
            cfg!(target_os = "linux"),
            "{}",
            f.advice
        );
    }

    #[test]
    fn key_held_from_the_sweep_into_the_next_step_is_not_unreleased() {
        // The sweep ends on the last key's press; the user holds that key
        // (Space) on into the hold step. Down, not stuck, not a fault.
        let mut t = AutoTest::new(InputSource::Evdev);
        t.set_expected_keys(vec![KeyCode(57)]);
        let t0 = Instant::now();
        t.start_at(t0);
        t.tick_at(t0 + IDLE_DURATION);
        let mut parts = ctx_parts();
        let space = evt(57, KeyEventType::Press, t0 + Duration::from_secs(4), 0);
        t.process_event(&space);
        parts.2.process_event(&space);
        parts.8.process_event(&space);
        t.tick_at(t0 + Duration::from_secs(4));
        assert_eq!(t.current_step(), Some(StepKind::Hold));
        let findings = t.analyze(&ctx!(parts));
        assert!(!findings.iter().any(Finding::is_issue), "{:?}", findings);

        // A key jammed in the sweep is one Stuck key finding, not two
        let jammed = Instant::now() - Duration::from_secs(5);
        let mut parts = ctx_parts();
        let backspace = evt(14, KeyEventType::Press, jammed, 0);
        parts.2.process_event(&backspace);
        parts.8.process_event(&backspace);
        t.process_event(&evt(
            14,
            KeyEventType::Press,
            t0 + Duration::from_secs(5),
            0,
        ));
        t.abort();
        let findings = t.analyze(&ctx!(parts));
        let issues: Vec<&str> = findings
            .iter()
            .filter(|f| f.is_issue())
            .map(|f| f.category)
            .collect();
        assert_eq!(issues, vec!["Stuck key"]);
    }

    #[test]
    fn summary_is_red_only_for_a_critical_finding() {
        let warning = Finding::new(Severity::Warning, "Connection", "t", "", "");
        let critical = Finding::new(Severity::Critical, "Dead keys", "t", "", "");
        let info = Finding::new(Severity::Info, "Polling rate", "t", "", "");
        assert_eq!(issues_status(std::slice::from_ref(&info)), ResultStatus::Ok);
        assert_eq!(
            issues_status(&[info.clone(), warning.clone()]),
            ResultStatus::Warning
        );
        assert_eq!(
            issues_status(&[warning.clone(), critical, info]),
            ResultStatus::Error
        );
        // The finished run's status row follows the same rule
        let mut t = AutoTest::new(InputSource::Evdev);
        t.start();
        t.abort();
        let status =
            |rows: Vec<TestResult>| rows.into_iter().find(|r| r.label == "Status").unwrap();
        let row = status(t.results(&[warning]));
        assert_eq!(row.status, ResultStatus::Warning);
        assert!(row.value.contains("1 issue(s)"), "{}", row.value);
        assert_eq!(status(t.results(&[])).status, ResultStatus::Ok);
    }

    /// Start a run and skip ahead to `step`, with no keys expected in the sweep.
    fn run_at_step(source: InputSource, step: StepKind) -> (AutoTest, Instant) {
        let mut t = AutoTest::new(source);
        t.set_expected_keys(vec![]);
        let t0 = Instant::now();
        t.start_at(t0);
        while t.current_step() != Some(step) {
            t.skip_step_at(t0);
        }
        (t, t0)
    }

    /// Press `keys` together at `at`, then release them all 500 ms later.
    fn chord(t: &mut AutoTest, keys: &[u16], at: Instant) {
        for (i, k) in keys.iter().enumerate() {
            let when = at + Duration::from_millis(i as u64 * 10);
            t.process_event(&evt(*k, KeyEventType::Press, when, 0));
        }
        for (i, k) in keys.iter().enumerate() {
            let when = at + Duration::from_millis(500 + i as u64 * 10);
            t.process_event(&evt(*k, KeyEventType::Release, when, 0));
        }
    }

    #[test]
    fn rollover_step_records_peak_and_completes_after_release() {
        let (mut t, t0) = run_at_step(InputSource::Evdev, StepKind::Rollover);
        let s = t0 + Duration::from_secs(4);
        chord(&mut t, &[30, 31, 32, 33, 34, 35, 36], s);
        // Not yet: needs 1s after the peak
        assert!(!t.tick_at(s + Duration::from_millis(600)));
        assert_eq!(t.current_step(), Some(StepKind::Rollover));
        assert!(!t.tick_at(s + Duration::from_millis(1600)));
        assert_eq!(t.current_step(), Some(StepKind::RapidTap));

        let parts = ctx_parts();
        let findings = t.analyze(&ctx!(parts));
        let r = findings
            .iter()
            .find(|f| f.category == "Rollover")
            .expect("rollover finding");
        assert_eq!(r.severity, Severity::Info);
        assert!(r.title.contains("7 keys"), "{}", r.title);
    }

    #[test]
    fn rollover_low_first_attempt_keeps_the_step_open() {
        let (mut t, t0) = run_at_step(InputSource::Evdev, StepKind::Rollover);
        let s = t0 + Duration::from_secs(4);
        // A tentative first try with two keys, then a pause
        chord(&mut t, &[30, 31], s);
        assert!(!t.tick_at(s + Duration::from_secs(3)));
        assert_eq!(t.current_step(), Some(StepKind::Rollover));
        // The real attempt
        let s = s + Duration::from_secs(4);
        chord(&mut t, &[30, 31, 32, 33, 34, 35], s);
        assert!(!t.tick_at(s + Duration::from_secs(2)));
        assert_eq!(t.current_step(), Some(StepKind::RapidTap));
        let parts = ctx_parts();
        let findings = t.analyze(&ctx!(parts));
        assert!(!findings.iter().any(Finding::is_issue), "{:?}", findings);
    }

    #[test]
    fn rollover_timeout_with_low_peak_is_a_conditional_warning() {
        let (mut t, t0) = run_at_step(InputSource::Evdev, StepKind::Rollover);
        chord(&mut t, &[30, 31, 32, 33], t0 + Duration::from_secs(1));
        t.tick_at(t0 + StepKind::Rollover.timeout());
        assert_eq!(
            t.outcomes().last(),
            Some(&(StepKind::Rollover, StepOutcome::TimedOut))
        );
        let parts = ctx_parts();
        let findings = t.analyze(&ctx!(parts));
        let r = findings.iter().find(|f| f.category == "Rollover").unwrap();
        // Could be the keyboard or the user: worth a look, never "Critical",
        // and worded so pressing few keys is not called a keyboard limit.
        assert_eq!(r.severity, Severity::Warning);
        assert!(r.title.contains("4 key(s)"), "{}", r.title);
        assert!(r.advice.contains("If you held fewer, this is not a fault"));
    }

    #[test]
    fn rollover_low_peak_says_when_modifiers_were_not_counted() {
        // The Windows console never reports Shift/Ctrl/Alt/Win, so a user
        // holding them with letters sees a lower count than they pressed
        let r = rollover_finding(Some(StepOutcome::TimedOut), 3, false).unwrap();
        assert_eq!(r.severity, Severity::Warning);
        assert!(r.detail.contains("were not counted"), "{}", r.detail);
        let r = rollover_finding(Some(StepOutcome::TimedOut), 3, true).unwrap();
        assert!(!r.detail.contains("not counted"), "{}", r.detail);
    }

    #[test]
    fn rollover_skipped_step_is_not_judged() {
        let (mut t, t0) = run_at_step(InputSource::Evdev, StepKind::Rollover);
        chord(&mut t, &[30, 31], t0 + Duration::from_secs(1));
        t.skip_step_at(t0 + Duration::from_secs(3));
        let parts = ctx_parts();
        let findings = t.analyze(&ctx!(parts));
        let r = findings.iter().find(|f| f.category == "Rollover").unwrap();
        assert_eq!(r.severity, Severity::Info);
        assert!(r.title.contains("not finished"));
    }

    #[test]
    fn rapid_tap_in_press_only_terminal_completes_without_judging_chatter() {
        let mut t = AutoTest::new(InputSource::TerminalPressOnly);
        t.set_expected_keys(vec![]);
        let t0 = Instant::now();
        t.start_at(t0);
        t.tick_at(t0 + IDLE_DURATION); // -> Sweep (empty)
        t.tick_at(t0 + IDLE_DURATION); // -> RapidTap
        assert_eq!(t.current_step(), Some(StepKind::RapidTap));

        let s = t0 + Duration::from_secs(4);
        // Normal taps 80ms apart, with one 2ms double-fire
        let mut at = s;
        for i in 0..10 {
            t.process_event(&evt(30, KeyEventType::Press, at, 80_000));
            if i == 5 {
                at += Duration::from_millis(2);
                t.process_event(&evt(30, KeyEventType::Press, at, 2_000));
            }
            at += Duration::from_millis(80);
        }
        assert!(!t.tick_at(s + Duration::from_secs(1)));
        assert!(t.tick_at(s + RAPID_TAP_DURATION));
        assert_eq!(t.state(), AutoTestState::Complete);
        assert_eq!(t.progress(), 1.0);

        let parts = ctx_parts();
        let findings = t.analyze(&ctx!(parts));
        // Read-time timestamps cannot show chatter: say so, do not guess
        let chatter = findings
            .iter()
            .find(|f| f.category == "Key chatter")
            .expect("chatter finding");
        assert_eq!(chatter.severity, Severity::Info);
        assert!(chatter.title.contains("cannot be measured"));
        // Press-only terminals get the capture limitation, as information
        let limited = findings
            .iter()
            .find(|f| f.category == "Limited capture")
            .expect("limited capture finding");
        assert_eq!(limited.severity, Severity::Info);
        assert!(!findings.iter().any(Finding::is_issue), "{:?}", findings);
        // The listener counts every tap; only the timing is approximate
        let rapid = findings.iter().find(|f| f.category == "Rapid tap").unwrap();
        assert!(rapid.title.starts_with("11 presses"), "{}", rapid.title);
        assert!(rapid.detail.contains("approximate"), "{}", rapid.detail);
        assert!(!rapid.detail.contains("undercounted"), "{}", rapid.detail);
        // Sorted most severe first
        assert!(findings.windows(2).all(|w| w[0].severity >= w[1].severity));
    }

    /// Feed `events` to the run and to the bounce test, as App does.
    fn feed_both(t: &mut AutoTest, bounce: &mut HoldReleaseTest, events: &[KeyEvent]) {
        for e in events {
            t.process_event(e);
            bounce.process_event(e);
        }
    }

    /// Taps of `key` every 100 ms (40 ms down) from `at`; the taps listed in
    /// `chatter` re-close the switch 6 ms after it opened.
    fn taps(key: u16, at: Instant, count: u64, chatter: &[u64]) -> Vec<KeyEvent> {
        let ms = Duration::from_millis;
        let mut events = Vec::new();
        for i in 0..count {
            let down = at + ms(i * 100);
            events.push(evt(key, KeyEventType::Press, down, 0));
            events.push(evt(key, KeyEventType::Release, down + ms(40), 0));
            if chatter.contains(&i) {
                events.push(evt(key, KeyEventType::Press, down + ms(46), 0));
                events.push(evt(key, KeyEventType::Release, down + ms(54), 0));
            }
        }
        events
    }

    #[test]
    fn rapid_tap_chatter_detected_with_evdev() {
        // The switch registers two of ten taps twice: a fault
        let (mut t, t0) = run_at_step(InputSource::Evdev, StepKind::RapidTap);
        let mut parts = ctx_parts();
        let s = t0 + Duration::from_secs(1);
        feed_both(&mut t, &mut parts.1, &taps(30, s, 10, &[4, 7]));
        let findings = t.analyze(&ctx!(parts));
        let bounce = findings
            .iter()
            .find(|f| f.category == "Switch bounce")
            .expect("bounce finding");
        assert_eq!(bounce.severity, Severity::Critical);
        assert_eq!(bounce.detail, "A (2 bounces)");
        // One fault, one finding
        assert_eq!(findings.iter().filter(|f| f.is_issue()).count(), 1);
    }

    #[test]
    fn a_single_bounce_is_a_conditional_warning() {
        // One doubled keystroke: a worn switch, or a wireless keyboard whose
        // delayed release arrived together with the next press
        let (mut t, t0) = run_at_step(InputSource::Evdev, StepKind::RapidTap);
        let mut parts = ctx_parts();
        let s = t0 + Duration::from_secs(1);
        feed_both(&mut t, &mut parts.1, &taps(30, s, 10, &[4]));
        let findings = t.analyze(&ctx!(parts));
        let bounce = findings
            .iter()
            .find(|f| f.category == "Switch bounce")
            .expect("bounce finding");
        assert_eq!(bounce.severity, Severity::Warning);
        assert_eq!(bounce.detail, "A (1 bounce)");
        assert!(bounce.advice.contains("wireless"), "{}", bounce.advice);
        assert!(bounce.advice.contains("if it bounces again"));
    }

    #[test]
    fn held_laptop_hotkey_is_not_chatter() {
        // A volume key with an atkbd force-release quirk held for a second:
        // typematic presses every 33 ms, each released by the driver a few
        // µs later. The key never reports its own release: not a fault.
        let (mut t, t0) = run_at_step(InputSource::Evdev, StepKind::Sweep);
        let mut parts = ctx_parts();
        let mut events = Vec::new();
        let mut at = t0 + Duration::from_secs(1);
        for i in 0..25u64 {
            events.push(evt(115, KeyEventType::Press, at, 0));
            events.push(evt(
                115,
                KeyEventType::Release,
                at + Duration::from_micros(5),
                0,
            ));
            at += if i == 0 {
                Duration::from_millis(250)
            } else {
                Duration::from_micros(33_333)
            };
        }
        feed_both(&mut t, &mut parts.1, &events);
        let findings = t.analyze(&ctx!(parts));
        assert!(!findings.iter().any(Finding::is_issue), "{:?}", findings);
    }

    #[test]
    fn fast_human_tapping_is_not_chatter() {
        // 12 taps/s, about as fast as one finger goes, quantized to a 125 Hz
        // grid (88 ms period, 40 ms down, 48 ms up), plus a quick double-tap
        // on another key (key up 32 ms, presses 72 ms apart)
        let (mut t, t0) = run_at_step(InputSource::Evdev, StepKind::RapidTap);
        let mut parts = ctx_parts();
        let s = t0 + Duration::from_secs(1);
        let ms = Duration::from_millis;
        let mut events = Vec::new();
        for i in 0..40u64 {
            let at = s + ms(i * 88);
            events.push(evt(30, KeyEventType::Press, at, 0));
            events.push(evt(30, KeyEventType::Release, at + ms(40), 0));
        }
        let d = s + Duration::from_secs(5);
        events.push(evt(31, KeyEventType::Press, d, 0));
        events.push(evt(31, KeyEventType::Release, d + ms(40), 0));
        events.push(evt(31, KeyEventType::Press, d + ms(72), 0));
        feed_both(&mut t, &mut parts.1, &events);
        let findings = t.analyze(&ctx!(parts));
        assert!(
            !findings.iter().any(|f| f.category == "Switch bounce"),
            "{:?}",
            findings
        );
    }

    #[test]
    fn chatter_not_judged_from_terminal_timestamps() {
        // A burst read in one go gets near-identical read-time timestamps
        let (mut t, t0) = run_at_step(InputSource::TerminalWithRelease, StepKind::RapidTap);
        let s = t0 + Duration::from_secs(1);
        for i in 0..4u64 {
            let at = s + Duration::from_micros(i * 300);
            t.process_event(&evt(30, KeyEventType::Press, at, 0));
            t.process_event(&evt(
                30,
                KeyEventType::Release,
                at + Duration::from_micros(100),
                0,
            ));
        }
        t.abort();
        let parts = ctx_parts();
        let findings = t.analyze(&ctx!(parts));
        let chatter = findings
            .iter()
            .find(|f| f.category == "Key chatter")
            .unwrap();
        assert_eq!(chatter.severity, Severity::Info);
    }

    #[test]
    fn tap_rate_uses_the_measured_span() {
        assert_eq!(tap_rate(11, Duration::from_secs(1)), Some(10.0));
        assert_eq!(tap_rate(1, Duration::from_secs(1)), None);
        assert_eq!(tap_rate(5, Duration::ZERO), None);
    }

    #[test]
    fn rapid_tap_rate_reflects_an_aborted_step() {
        let (mut t, t0) = run_at_step(InputSource::Evdev, StepKind::RapidTap);
        let s = t0 + Duration::from_secs(1);
        // 11 presses 100 ms apart (10/s for one second), then abort
        for i in 0..11u64 {
            let at = s + Duration::from_millis(i * 100);
            t.process_event(&evt(30, KeyEventType::Press, at, 0));
            t.process_event(&evt(
                30,
                KeyEventType::Release,
                at + Duration::from_millis(40),
                0,
            ));
        }
        t.abort();
        let parts = ctx_parts();
        let findings = t.analyze(&ctx!(parts));
        let f = findings.iter().find(|f| f.category == "Rapid tap").unwrap();
        assert!(
            f.title.contains("11 presses in 1.0s (10.0 per second)"),
            "{}",
            f.title
        );
    }

    #[test]
    fn clean_run_reports_no_issues() {
        let mut t = AutoTest::new(InputSource::Evdev);
        t.set_expected_keys(vec![KeyCode(30)]);
        let t0 = Instant::now();
        t.start_at(t0);
        t.tick_at(t0 + IDLE_DURATION); // -> Sweep
        let s = t0 + Duration::from_secs(4);
        t.process_event(&evt(30, KeyEventType::Press, s, 0));
        t.process_event(&evt(
            30,
            KeyEventType::Release,
            s + Duration::from_millis(60),
            0,
        ));
        t.tick_at(s + Duration::from_millis(100)); // -> Hold
        t.process_event(&evt(30, KeyEventType::Press, s + Duration::from_secs(1), 0));
        t.process_event(&evt(
            30,
            KeyEventType::Release,
            s + Duration::from_secs(2),
            0,
        ));
        t.tick_at(s + Duration::from_secs(2)); // -> Rollover
        let r = s + Duration::from_secs(3);
        for (i, k) in (30u16..37).enumerate() {
            t.process_event(&evt(
                k,
                KeyEventType::Press,
                r + Duration::from_millis(i as u64 * 20),
                0,
            ));
        }
        for (i, k) in (30u16..37).enumerate() {
            t.process_event(&evt(
                k,
                KeyEventType::Release,
                r + Duration::from_millis(400 + i as u64 * 20),
                0,
            ));
        }
        t.tick_at(r + Duration::from_secs(2)); // -> RapidTap
        let q = r + Duration::from_secs(3);
        for i in 0..20u64 {
            t.process_event(&evt(
                30,
                KeyEventType::Press,
                q + Duration::from_millis(i * 70),
                70_000,
            ));
            t.process_event(&evt(
                30,
                KeyEventType::Release,
                q + Duration::from_millis(i * 70 + 30),
                30_000,
            ));
        }
        assert!(t.tick_at(q + RAPID_TAP_DURATION));
        assert_eq!(t.state(), AutoTestState::Complete);
        assert!(t.outcomes().iter().all(|(_, o)| *o == StepOutcome::Passed));

        let parts = ctx_parts();
        let findings = t.analyze(&ctx!(parts));
        assert!(!findings.iter().any(Finding::is_issue), "{:?}", findings);
        assert_eq!(findings[0].category, "Summary");
        assert!(findings[0].title.contains("no issues"));
        assert!(findings
            .iter()
            .any(|f| f.category == "Rollover" && f.title.contains("7 keys")));
        // 20 presses 70 ms apart: 19 intervals over 1.33 s
        assert!(findings
            .iter()
            .any(|f| f.category == "Rapid tap" && f.title.contains("14.3 per second")));
    }

    #[test]
    fn analyze_surfaces_continuous_test_findings() {
        // Bounce + stuck keys from the other tests are reported even without a run
        let mut parts = ctx_parts();
        let t0 = Instant::now();
        parts.1.process_event(&evt(30, KeyEventType::Press, t0, 0));
        parts.1.process_event(&evt(
            30,
            KeyEventType::Release,
            t0 + Duration::from_millis(1),
            0,
        ));
        parts.1.process_event(&evt(
            30,
            KeyEventType::Press,
            t0 + Duration::from_millis(2),
            0,
        ));
        let t = AutoTest::new(InputSource::Evdev);
        let findings = t.analyze(&ctx!(parts));
        assert!(findings.iter().any(|f| f.category == "Switch bounce"));

        // The same burst from a terminal is read-time stamps, not a bounce
        let t = AutoTest::new(InputSource::TerminalWithRelease);
        let findings = t.analyze(&ctx!(parts));
        assert!(!findings.iter().any(|f| f.category == "Switch bounce"));
        assert!(!findings.iter().any(Finding::is_issue), "{:?}", findings);
    }

    #[test]
    fn analyze_reports_polling_rate_from_quantized_timestamps() {
        let mut parts = ctx_parts();
        let t0 = Instant::now();
        // 30 presses rolling over four keys, exactly 8ms*k apart -> 125 Hz grid
        let mut at = t0;
        for i in 0..30u64 {
            let gap = Duration::from_millis(8 * (3 + i % 4)); // 24..56ms
            at += gap;
            let key = 30 + (i % 4) as u16;
            parts
                .0
                .process_event(&evt(key, KeyEventType::Press, at, gap.as_micros() as u64));
        }
        let t = AutoTest::new(InputSource::Evdev);
        let findings = t.analyze(&ctx!(parts));
        let p = findings
            .iter()
            .find(|f| f.category == "Polling rate")
            .expect("polling finding");
        assert!(p.title.contains("8 ms"), "{}", p.title);
        assert!(p.title.contains("125 Hz"), "{}", p.title);
    }

    #[test]
    fn abort_keeps_observations() {
        let mut t = AutoTest::new(InputSource::Evdev);
        let t0 = Instant::now();
        t.start_at(t0);
        t.process_event(&press(KeyCode(30)));
        t.tick_at(t0 + IDLE_DURATION);
        t.abort();
        assert_eq!(t.state(), AutoTestState::Aborted);
        assert!(!t.is_running());
        let parts = ctx_parts();
        let findings = t.analyze(&ctx!(parts));
        assert!(findings.iter().any(|f| f.category == "Phantom input"));
        let rows = t.results(&findings);
        assert!(rows.iter().any(|r| r.value.contains("Aborted")));
    }

    #[test]
    fn reset_returns_to_idle() {
        let mut t = AutoTest::new(InputSource::Evdev);
        t.start();
        t.process_event(&press(KeyCode(30)));
        t.reset();
        assert_eq!(t.state(), AutoTestState::Idle);
        assert!(t.steps().is_empty());
        assert!(t.verified_keys().is_empty());
    }

    #[test]
    fn results_render_each_state() {
        let mut t = AutoTest::new(InputSource::Evdev);
        let idle = t.results(&[]);
        assert!(idle.iter().any(|r| r.value.contains("Not started")));
        t.start();
        let running = t.results(&[]);
        assert!(running.iter().any(|r| r.label == "Do this"));
        assert!(running.iter().any(|r| r.value.contains("Ctrl+N")));
    }

    #[test]
    fn expected_keys_cover_layout_and_arrows() {
        let keys = expected_keys_for_layout(KeyboardLayout::Ansi);
        assert!(keys.contains(&KeyCode(30))); // A
        assert!(keys.contains(&KeyCode(57))); // Space
        assert!(keys.contains(&KeyCode(103))); // Up
        assert!(!keys.contains(&KeyCode(86))); // ISO key absent on ANSI
        let iso = expected_keys_for_layout(KeyboardLayout::Iso);
        assert!(iso.contains(&KeyCode(86)));
        // No duplicates
        let set: HashSet<_> = keys.iter().collect();
        assert_eq!(set.len(), keys.len());
    }

    #[test]
    fn finding_rows_carry_severity_status() {
        let f = Finding::new(Severity::Critical, "Test", "title", "detail", "advice");
        let rows = f.to_results();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].status, ResultStatus::Error);
        assert_eq!(rows[0].label, "Test");
        assert!(rows[2].value.contains("advice"));
        assert!(f.is_issue());
        assert!(!Finding::new(Severity::Info, "x", "", "", "").is_issue());
    }

    #[test]
    fn start_keystroke_read_after_start_is_not_phantom() {
        // With evdev the 'A' that started the test is read on the next loop
        // iteration, but its kernel timestamp predates the start.
        let mut t = AutoTest::new(InputSource::Evdev);
        let pressed = Instant::now();
        let t0 = pressed + Duration::from_millis(5);
        t.start_at(t0);
        t.process_event(&evt(42, KeyEventType::Press, pressed, 0));
        t.process_event(&evt(30, KeyEventType::Press, pressed, 0));
        t.process_event(&evt(
            30,
            KeyEventType::Release,
            t0 + Duration::from_millis(80),
            0,
        ));
        t.process_event(&evt(
            42,
            KeyEventType::Release,
            t0 + Duration::from_millis(90),
            0,
        ));
        t.tick_at(t0 + IDLE_DURATION);
        let parts = ctx_parts();
        let findings = t.analyze(&ctx!(parts));
        assert!(
            !findings.iter().any(|f| f.category == "Phantom input"),
            "{:?}",
            findings
        );

        // A press that really happens during the idle step is still caught
        let mut t = AutoTest::new(InputSource::Evdev);
        t.start_at(t0);
        t.process_event(&evt(
            31,
            KeyEventType::Press,
            t0 + Duration::from_secs(1),
            0,
        ));
        t.tick_at(t0 + IDLE_DURATION);
        let findings = t.analyze(&ctx!(parts));
        assert!(findings.iter().any(|f| f.category == "Phantom input"));
    }

    #[test]
    fn skip_keystroke_read_after_the_skip_stays_out_of_the_next_step() {
        // With evdev the Ctrl+N that skipped the rollover step is read on the
        // next loop iteration. Its presses predate the rapid-tap step: they
        // must not start its timer or count as taps.
        let (mut t, t0) = run_at_step(InputSource::Evdev, StepKind::Rollover);
        let skip = t0 + Duration::from_secs(10);
        t.skip_step_at(skip);
        assert_eq!(t.current_step(), Some(StepKind::RapidTap));
        let ms = Duration::from_millis;
        t.process_event(&evt(29, KeyEventType::Press, skip - ms(60), 0));
        t.process_event(&evt(49, KeyEventType::Press, skip - ms(20), 0));
        t.process_event(&evt(49, KeyEventType::Release, skip + ms(50), 0));
        t.process_event(&evt(29, KeyEventType::Release, skip + ms(80), 0));
        // The user reads the instruction, then taps 11 times in one second
        let s = skip + Duration::from_secs(3);
        assert!(!t.tick_at(s));
        for i in 0..11u64 {
            t.process_event(&evt(30, KeyEventType::Press, s + ms(i * 100), 0));
            t.process_event(&evt(30, KeyEventType::Release, s + ms(i * 100 + 40), 0));
        }
        // The 5 s window runs from the first tap
        assert!(!t.tick_at(s + Duration::from_secs(2)));
        assert!(t.tick_at(s + RAPID_TAP_DURATION));
        let parts = ctx_parts();
        let findings = t.analyze(&ctx!(parts));
        let f = findings.iter().find(|f| f.category == "Rapid tap").unwrap();
        assert!(
            f.title.contains("11 presses in 1.0s (10.0 per second)"),
            "{}",
            f.title
        );
    }

    #[test]
    fn expected_keys_follow_what_the_input_source_reports() {
        let ansi = expected_keys_for_layout(KeyboardLayout::Ansi);
        let modifiers_and_caps: Vec<KeyCode> = ansi
            .iter()
            .copied()
            .filter(|k| keymap::is_modifier(*k) || *k == KeyCode(58))
            .collect();
        assert_eq!(modifiers_and_caps.len(), 8);

        // Evdev sees everything
        let mut t = AutoTest::new(InputSource::Evdev);
        t.set_expected_keys(ansi.clone());
        assert_eq!(t.expected_keys(), ansi.as_slice());
        assert!(t.unreportable_keys().is_empty());

        // A legacy terminal (and the Windows console) never reports keys
        // that type nothing; App sets the keys before the source
        let mut t = AutoTest::new(InputSource::Evdev);
        t.set_expected_keys(ansi.clone());
        t.set_input_source(InputSource::TerminalPressOnly);
        for k in &modifiers_and_caps {
            assert!(!t.expected_keys().contains(k), "{:?}", k);
        }
        assert_eq!(t.unreportable_keys(), modifiers_and_caps);
        assert_eq!(t.expected_keys().len(), ansi.len() - 8);
        for k in [30u16, 57, 28, 14, 15, 41, 43, 103, 105, 106, 108] {
            assert!(t.expected_keys().contains(&KeyCode(k)), "{}", k);
        }

        // The kitty protocol reports modifiers and CapsLock too
        for k in &modifiers_and_caps {
            assert!(terminal_can_report(*k, true), "{:?}", k);
            assert!(!terminal_can_report(*k, false), "{:?}", k);
        }

        // No terminal can identify the ISO and JIS extra keys
        for k in [86u16, 92, 93, 94] {
            assert!(!terminal_can_report(KeyCode(k), true), "{}", k);
            assert!(InputSource::Evdev.can_report(KeyCode(k)));
        }
        let mut t = AutoTest::new(InputSource::TerminalWithRelease);
        t.set_expected_keys(expected_keys_for_layout(KeyboardLayout::Iso));
        assert!(!t.expected_keys().contains(&KeyCode(86)));
        assert!(t.unreportable_keys().contains(&KeyCode(86)));
    }

    #[test]
    fn terminal_sweep_completes_without_unreportable_keys() {
        let mut t = AutoTest::new(InputSource::TerminalPressOnly);
        t.set_expected_keys(expected_keys_for_layout(KeyboardLayout::Ansi));
        let t0 = Instant::now();
        t.start_at(t0);
        t.tick_at(t0 + IDLE_DURATION);
        for k in t.expected_keys().to_vec() {
            t.process_event(&evt(
                k.0,
                KeyEventType::Press,
                t0 + Duration::from_secs(4),
                0,
            ));
        }
        t.tick_at(t0 + Duration::from_secs(5));
        assert_eq!(t.current_step(), Some(StepKind::RapidTap));
        t.skip_step_at(t0 + Duration::from_secs(6));
        let parts = ctx_parts();
        let findings = t.analyze(&ctx!(parts));
        assert!(!findings.iter().any(Finding::is_issue), "{:?}", findings);
        // The keys that could not be checked are listed, not passed silently
        let f = findings
            .iter()
            .find(|f| f.category == "Not checked")
            .unwrap();
        assert_eq!(f.severity, Severity::Info);
        assert!(f.detail.contains("LeftShift") && f.detail.contains("CapsLock"));
    }

    #[test]
    fn terminal_sweep_timeout_is_never_called_dead() {
        let mut t = AutoTest::new(InputSource::TerminalPressOnly);
        // A letter, a punctuation key (another character on non-US layouts)
        t.set_expected_keys(vec![KeyCode(30), KeyCode(39), KeyCode(31)]);
        let t0 = Instant::now();
        t.start_at(t0);
        t.tick_at(t0 + IDLE_DURATION);
        t.process_event(&evt(
            31,
            KeyEventType::Press,
            t0 + Duration::from_secs(4),
            0,
        ));
        t.tick_at(t0 + IDLE_DURATION + StepKind::Sweep.timeout());
        assert_eq!(t.outcome_of(StepKind::Sweep), Some(StepOutcome::TimedOut));
        let parts = ctx_parts();
        let findings = t.analyze(&ctx!(parts));
        assert!(
            !findings.iter().any(|f| f.severity == Severity::Critical),
            "{:?}",
            findings
        );
        let letters = findings
            .iter()
            .find(|f| f.category == "Not registered" && f.severity == Severity::Warning)
            .expect("letter warning");
        assert_eq!(letters.detail, "Keys: A");
        let punct = findings
            .iter()
            .find(|f| f.category == "Not registered" && f.severity == Severity::Info)
            .expect("punctuation info");
        assert_eq!(punct.detail, "Keys: Semicolon");
    }

    #[test]
    fn evdev_sweep_timeout_separates_keys_not_every_keyboard_has() {
        let mut t = AutoTest::new(InputSource::Evdev);
        t.set_expected_keys(vec![KeyCode(30), KeyCode(97), KeyCode(86), KeyCode(125)]);
        let t0 = Instant::now();
        t.start_at(t0);
        t.tick_at(t0 + IDLE_DURATION);
        t.tick_at(t0 + IDLE_DURATION + StepKind::Sweep.timeout());
        let parts = ctx_parts();
        let findings = t.analyze(&ctx!(parts));
        let dead = findings.iter().find(|f| f.category == "Dead keys").unwrap();
        assert_eq!(dead.severity, Severity::Critical);
        assert_eq!(dead.detail, "Keys: A");
        let optional = findings
            .iter()
            .find(|f| f.category == "Not registered")
            .unwrap();
        assert_eq!(optional.severity, Severity::Warning);
        assert_eq!(optional.detail, "Keys: IsoBackslash, LeftMeta, RightCtrl");

        // A healthy laptop without Right Ctrl, or a gaming keyboard with its
        // Windows key locked: no critical finding
        let mut t = AutoTest::new(InputSource::Evdev);
        t.set_expected_keys(vec![KeyCode(30), KeyCode(97), KeyCode(125)]);
        t.start_at(t0);
        t.tick_at(t0 + IDLE_DURATION);
        t.process_event(&evt(
            30,
            KeyEventType::Press,
            t0 + Duration::from_secs(4),
            0,
        ));
        t.tick_at(t0 + IDLE_DURATION + StepKind::Sweep.timeout());
        let findings = t.analyze(&ctx!(parts));
        assert!(
            !findings.iter().any(|f| f.severity == Severity::Critical),
            "{:?}",
            findings
        );
    }

    #[test]
    fn released_long_hold_is_info_not_stuck() {
        // Holding W for 3 s (e.g. in a game), then letting go, is normal
        let mut parts = ctx_parts();
        let past = Instant::now() - Duration::from_secs(3);
        parts
            .2
            .process_event(&evt(17, KeyEventType::Press, past, 0));
        assert_eq!(parts.2.check_stuck_keys(), vec![KeyCode(17)]);
        parts
            .2
            .process_event(&evt(17, KeyEventType::Release, Instant::now(), 0));
        let t = AutoTest::new(InputSource::Evdev);
        let findings = t.analyze(&ctx!(parts));
        assert!(!findings.iter().any(Finding::is_issue), "{:?}", findings);
        let f = findings.iter().find(|f| f.category == "Long hold").unwrap();
        assert_eq!(f.severity, Severity::Info);
        assert!(f.detail.contains('W'));
    }

    #[test]
    fn key_still_held_past_threshold_is_stuck() {
        let mut parts = ctx_parts();
        let past = Instant::now() - Duration::from_secs(5);
        parts
            .2
            .process_event(&evt(17, KeyEventType::Press, past, 0));
        parts.2.check_stuck_keys();
        let t = AutoTest::new(InputSource::Evdev);
        let findings = t.analyze(&ctx!(parts));
        let f = findings.iter().find(|f| f.category == "Stuck key").unwrap();
        assert_eq!(f.severity, Severity::Critical);
        assert!(f.detail.starts_with("W (5."), "{}", f.detail);
        // Not reported twice as a long hold
        assert!(!findings.iter().any(|f| f.category == "Long hold"));

        // A key held briefly is fine
        let mut parts = ctx_parts();
        parts
            .2
            .process_event(&evt(17, KeyEventType::Press, Instant::now(), 0));
        let findings = t.analyze(&ctx!(parts));
        assert!(!findings.iter().any(|f| f.category == "Stuck key"));
    }

    #[test]
    fn key_still_down_is_never_called_released() {
        // W was flagged at the 2 s threshold, then the hold step relaxed the
        // threshold to 15 s while W is still down: it is not a released long
        // hold (and, the user being asked to hold keys, not stuck either)
        let mut parts = ctx_parts();
        let past = Instant::now() - Duration::from_secs(3);
        parts
            .2
            .process_event(&evt(17, KeyEventType::Press, past, 0));
        assert_eq!(parts.2.check_stuck_keys(), vec![KeyCode(17)]);
        parts.2.set_threshold(15_000);
        let (t, _) = run_at_step(InputSource::Evdev, StepKind::Hold);
        let findings = t.analyze(&ctx!(parts));
        assert!(
            !findings
                .iter()
                .any(|f| f.category == "Long hold" || f.category == "Stuck key"),
            "{:?}",
            findings
        );

        // Once it is let go it is listed as a long hold, with its duration
        parts
            .2
            .process_event(&evt(17, KeyEventType::Release, Instant::now(), 0));
        let findings = t.analyze(&ctx!(parts));
        let f = findings.iter().find(|f| f.category == "Long hold").unwrap();
        assert!(f.detail.starts_with("W (1x, longest 3."), "{}", f.detail);
    }

    #[test]
    fn keys_held_on_purpose_are_not_stuck() {
        for step in [StepKind::Hold, StepKind::Rollover] {
            let mut parts = ctx_parts();
            let past = Instant::now() - Duration::from_secs(3);
            for k in [30u16, 31, 32] {
                parts.2.process_event(&evt(k, KeyEventType::Press, past, 0));
            }
            let (mut t, t0) = run_at_step(InputSource::Evdev, step);
            let findings = t.analyze(&ctx!(parts));
            assert!(
                !findings.iter().any(|f| f.category == "Stuck key"),
                "{:?}",
                findings
            );
            // Once those steps are over, keys that are still down are stuck
            while matches!(t.current_step(), Some(StepKind::Hold | StepKind::Rollover)) {
                t.skip_step_at(t0);
            }
            let findings = t.analyze(&ctx!(parts));
            assert!(findings.iter().any(|f| f.category == "Stuck key"));
        }
    }

    #[test]
    fn keyboard_disconnect_is_an_issue_and_names_the_device() {
        let parts = ctx_parts();
        let t = AutoTest::new(InputSource::Evdev);
        // Nothing dropped: no connection finding, a clean summary
        let findings = t.analyze(&ctx!(parts));
        assert!(!findings.iter().any(|f| f.category == "Connection"));
        assert!(!findings.iter().any(Finding::is_issue));

        let devices = vec!["Logitech USB Receiver".to_string()];
        let findings = t.analyze(&ctx!(parts, 2, &devices));
        let f = findings
            .iter()
            .find(|f| f.category == "Connection")
            .expect("connection finding");
        assert_eq!(f.severity, Severity::Warning);
        assert!(f.title.contains("2 time(s)"), "{}", f.title);
        assert!(
            f.detail.starts_with("Logitech USB Receiver. "),
            "{}",
            f.detail
        );
        // Worded as a condition: an unplug on purpose looks the same
        assert!(f.advice.starts_with("If you did not unplug it"));
        // The summary no longer says nothing was found
        assert!(!findings.iter().any(|f| f.category == "Summary"));
    }

    #[test]
    fn progress_bar_shape() {
        assert_eq!(progress_bar(0.0, 4), "[----]");
        assert_eq!(progress_bar(0.5, 4), "[##--]");
        assert_eq!(progress_bar(1.0, 4), "[####]");
        assert_eq!(progress_bar(7.0, 4), "[####]");
    }
}
