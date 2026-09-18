//! Automatic diagnostic test
//!
//! Drives a short guided sequence of steps and then analyses everything the
//! other tests collected to produce a ranked list of [`Finding`]s, so the user
//! does not have to visit every tab and interpret raw numbers.
//!
//! The steps are:
//!
//! 1. **Idle** – hands off the keyboard; any event is a phantom key press or a
//!    key that is already stuck.
//! 2. **Sweep** – press every key of the main block once; keys that never
//!    register are reported as dead, keys that never release as stuck.
//! 3. **Hold** – hold one key for about a second; verifies release reporting.
//! 4. **Rollover** – hold as many keys as possible; measures N-key rollover.
//! 5. **Rapid tap** – tap one key quickly; looks for switch chatter and
//!    infers the polling interval from timestamp quantization.
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
use crate::keyboard::{keymap, KeyCode, KeyEvent, KeyEventType, KeyboardState};
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
            Self::Hold => "Hold any single key for about one second, then release it.",
            Self::Rollover => "Press and hold as many keys as you can at once, then release all.",
            Self::RapidTap => "Tap one key as fast as you can for a few seconds.",
        }
    }

    /// How long the step may run before it is considered timed out.
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
/// Same-key press gaps below this cannot be produced by a finger: chatter.
pub const CHATTER_GAP_US: u64 = 15_000;
/// Rollover considered good (typical gaming/mechanical keyboards).
pub const GOOD_ROLLOVER: usize = 6;

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
    /// Keys the sweep expects to see
    expected_keys: Vec<KeyCode>,
    /// Keys pressed during the sweep
    seen_keys: HashSet<KeyCode>,
    /// Keys released during the sweep
    released_keys: HashSet<KeyCode>,
    /// Press timestamps for hold measurement
    press_times: HashMap<KeyCode, Instant>,
    /// Longest completed hold in the hold step
    longest_hold: Option<(KeyCode, Duration)>,
    /// Keys currently down during the rollover step
    rollover_pressed: HashSet<KeyCode>,
    /// Peak simultaneous keys during the rollover step
    rollover_max: usize,
    /// When the rollover peak was last raised
    rollover_peak_at: Option<Instant>,
    /// First press in the rapid-tap step
    rapid_started: Option<Instant>,
    /// Presses recorded in the rapid-tap step
    rapid_presses: u32,
    /// Inter-press gaps (µs) recorded in the rapid-tap step
    rapid_intervals: Vec<u64>,
    /// Last press time per key (for same-key gap / chatter detection)
    last_press_by_key: HashMap<KeyCode, Instant>,
    /// Suspiciously short same-key press gaps, per key
    chatter_suspects: HashMap<KeyCode, u32>,
    /// Presses that occurred in any step
    presses_during_run: u64,
}

impl AutoTest {
    /// Create an auto test for the given input source.
    pub fn new(input_source: InputSource) -> Self {
        Self {
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
            expected_keys: expected_keys_for_layout(KeyboardLayout::Ansi),
            seen_keys: HashSet::new(),
            released_keys: HashSet::new(),
            press_times: HashMap::new(),
            longest_hold: None,
            rollover_pressed: HashSet::new(),
            rollover_max: 0,
            rollover_peak_at: None,
            rapid_started: None,
            rapid_presses: 0,
            rapid_intervals: Vec::new(),
            last_press_by_key: HashMap::new(),
            chatter_suspects: HashMap::new(),
            presses_during_run: 0,
        }
    }

    /// Change the input source (e.g. once the main loop knows what it got).
    pub fn set_input_source(&mut self, source: InputSource) {
        self.input_source = source;
    }

    /// The input source this test assumes.
    pub fn input_source(&self) -> InputSource {
        self.input_source
    }

    /// Set the keys the sweep step expects to see.
    pub fn set_expected_keys(&mut self, keys: Vec<KeyCode>) {
        self.expected_keys = keys;
    }

    /// Keys the sweep step expects.
    pub fn expected_keys(&self) -> &[KeyCode] {
        &self.expected_keys
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
        self.released_keys.clear();
        self.press_times.clear();
        self.longest_hold = None;
        self.rollover_pressed.clear();
        self.rollover_max = 0;
        self.rollover_peak_at = None;
        self.rapid_started = None;
        self.rapid_presses = 0;
        self.rapid_intervals.clear();
        self.last_press_by_key.clear();
        self.chatter_suspects.clear();
        self.presses_during_run = 0;
    }

    /// Advance timers and step completion. Returns `true` when the whole
    /// sequence just finished on this call.
    pub fn tick(&mut self) -> bool {
        self.tick_at(Instant::now())
    }

    fn tick_at(&mut self, now: Instant) -> bool {
        let Some(step) = self.current_step() else {
            return false;
        };
        let elapsed = now.saturating_duration_since(self.step_started.unwrap_or(now));

        let passed = match step {
            StepKind::Idle => elapsed >= IDLE_DURATION,
            StepKind::Sweep => self.sweep_missing().is_empty(),
            StepKind::Hold => self.longest_hold.is_some(),
            StepKind::Rollover => {
                // Done once the user has let go after a real attempt.
                self.rollover_max >= 2
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
        } else if elapsed >= step.timeout() {
            self.finish_step(step, StepOutcome::TimedOut, now);
        } else {
            return false;
        }
        self.state == AutoTestState::Complete
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

        if event.event_type == KeyEventType::Press {
            self.presses_during_run += 1;
            // Same-key gap: a finger cannot re-press a key within a few ms.
            if let Some(last) = self.last_press_by_key.get(&event.key) {
                let gap_us = event.timestamp.saturating_duration_since(*last).as_micros() as u64;
                if gap_us < CHATTER_GAP_US {
                    *self.chatter_suspects.entry(event.key).or_insert(0) += 1;
                }
            }
            self.last_press_by_key.insert(event.key, event.timestamp);
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
            StepKind::Sweep => match event.event_type {
                KeyEventType::Press => {
                    self.seen_keys.insert(event.key);
                }
                KeyEventType::Release => {
                    self.released_keys.insert(event.key);
                }
            },
            StepKind::Hold => match event.event_type {
                KeyEventType::Press => {
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
                    if self.rapid_presses > 0 && event.delta_us > 0 {
                        self.rapid_intervals.push(event.delta_us);
                    }
                    self.rapid_presses += 1;
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

        // --- Input source limitations -------------------------------------
        if !releases {
            findings.push(Finding::new(
                Severity::Warning,
                "Limited capture",
                "Terminal reports key presses only",
                "Releases are synthesized, so hold, rollover, stuck-key and bounce \
                 checks are skipped.",
                if cfg!(target_os = "linux") {
                    "Run with sudo (or add your user to the 'input' group) for raw evdev \
                     access, or use a terminal with the kitty keyboard protocol."
                } else {
                    "Use a terminal that supports the kitty keyboard protocol \
                     (kitty, WezTerm, foot, Ghostty, Alacritty 0.13+)."
                },
            ));
        }

        // --- Idle step: phantom input -------------------------------------
        if self.step_was_run(StepKind::Idle) && !self.phantom_events.is_empty() {
            let keys = self
                .phantom_events
                .iter()
                .map(|(k, _)| *k)
                .collect::<HashSet<_>>();
            findings.push(Finding::new(
                Severity::Critical,
                "Phantom input",
                format!(
                    "{} event(s) arrived while nothing was pressed",
                    self.phantom_events.len()
                ),
                format!("Keys: {}", key_list(keys.into_iter(), 8)),
                "A key is stuck down, chattering, or something else is injecting input. \
                 Check for debris under the listed keys and for macro/remap software.",
            ));
        }

        // --- Sweep step: dead / unreleased keys ---------------------------
        match self.outcome_of(StepKind::Sweep) {
            Some(StepOutcome::TimedOut) => {
                let missing = self.sweep_missing();
                if !missing.is_empty() {
                    findings.push(Finding::new(
                        Severity::Critical,
                        "Dead keys",
                        format!("{} key(s) never registered", missing.len()),
                        format!("Keys: {}", key_list(missing.iter().copied(), 12)),
                        "If you did press them, the switch or its matrix trace is faulty. \
                         Clean or replace the switch; on laptops check the ribbon cable.",
                    ));
                }
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
        if releases && self.step_was_run(StepKind::Sweep) {
            let unreleased: Vec<KeyCode> = self
                .seen_keys
                .iter()
                .filter(|k| !self.released_keys.contains(k))
                .copied()
                .collect();
            // Keys still physically held while the user pressed the *next*
            // key are normal (e.g. Shift). Only flag keys the keyboard state
            // still thinks are down or that were never released at all.
            let still_down: Vec<KeyCode> = unreleased
                .iter()
                .filter(|k| ctx.keyboard_state.pressed_keys().contains(k))
                .copied()
                .collect();
            if !still_down.is_empty() {
                findings.push(Finding::new(
                    Severity::Warning,
                    "No release",
                    format!(
                        "{} key(s) registered a press but no release",
                        still_down.len()
                    ),
                    format!("Keys: {}", key_list(still_down.iter().copied(), 10)),
                    "The key may be physically stuck or the switch is not returning. \
                     Press it a few more times and watch the Sticky view.",
                ));
            }
        }

        // --- Stuck keys (continuous) --------------------------------------
        if releases {
            let flagged = ctx.stickiness.flagged_keys();
            if !flagged.is_empty() {
                let detail = flagged
                    .iter()
                    .take(6)
                    .map(|(k, d, n)| {
                        format!(
                            "{} ({:.1}s, {}x)",
                            keymap::get_key_info(*k).name,
                            d.as_secs_f64(),
                            n
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                findings.push(Finding::new(
                    Severity::Critical,
                    "Stuck key",
                    format!(
                        "{} key(s) stayed pressed past the {} ms threshold",
                        flagged.len(),
                        ctx.stickiness.threshold().as_millis()
                    ),
                    detail,
                    "Remove the keycap and clean the switch. If it persists, the switch \
                     or controller is faulty.",
                ));
            }
        }

        // --- Bounce / chatter (continuous) --------------------------------
        if releases {
            let bouncy = ctx.hold_release.bouncy_keys();
            if !bouncy.is_empty() {
                let detail = bouncy
                    .iter()
                    .take(6)
                    .map(|(k, n)| format!("{} ({} bounces)", keymap::get_key_info(*k).name, n))
                    .collect::<Vec<_>>()
                    .join(", ");
                findings.push(Finding::new(
                    Severity::Critical,
                    "Switch bounce",
                    format!(
                        "{} bounce(s) on {} key(s)",
                        ctx.hold_release.total_bounces(),
                        bouncy.len()
                    ),
                    detail,
                    "Contact bounce causes doubled characters. Clean the switch or \
                     increase firmware debounce; replace the switch if it continues.",
                ));
            }
        }
        if !self.chatter_suspects.is_empty() {
            let mut suspects: Vec<(KeyCode, u32)> = self
                .chatter_suspects
                .iter()
                .map(|(k, n)| (*k, *n))
                .collect();
            suspects.sort_by(|a, b| b.1.cmp(&a.1));
            let detail = suspects
                .iter()
                .take(6)
                .map(|(k, n)| format!("{} ({}x)", keymap::get_key_info(*k).name, n))
                .collect::<Vec<_>>()
                .join(", ");
            findings.push(Finding::new(
                Severity::Critical,
                "Key chatter",
                format!(
                    "Repeated presses under {} ms apart on {} key(s)",
                    CHATTER_GAP_US / 1000,
                    suspects.len()
                ),
                detail,
                "A finger cannot re-press that fast; the switch is registering one \
                 press as several. Clean or replace it.",
            ));
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
            Some(StepOutcome::TimedOut) => {
                findings.push(Finding::new(
                    Severity::Warning,
                    "Hold/release",
                    "No clean hold-then-release was observed",
                    format!(
                        "Expected a single key held for at least {} ms followed by its release.",
                        MIN_HOLD.as_millis()
                    ),
                    "If you did hold a key, releases are not reaching the app: check \
                     for remapping software or try the evdev input path.",
                ));
            }
            _ => {}
        }

        // --- Rollover -----------------------------------------------------
        if self.step_was_run(StepKind::Rollover) {
            let max = self.rollover_max;
            let (severity, title) = match max {
                0 => (
                    Severity::Info,
                    "Rollover step produced no key presses".to_string(),
                ),
                1..=2 => (
                    Severity::Critical,
                    format!("Only {} key(s) could be held at once", max),
                ),
                3..=5 => (
                    Severity::Warning,
                    format!("Limited rollover: {} keys at once", max),
                ),
                n => (Severity::Info, format!("Rollover OK: {} keys at once", n)),
            };
            findings.push(Finding::new(
                severity,
                "Rollover",
                title,
                if max > 0 {
                    format!(
                        "Peak of {} simultaneous keys during the rollover step.",
                        max
                    )
                } else {
                    String::new()
                },
                if max < GOOD_ROLLOVER && max > 0 {
                    "Low rollover blocks key combinations in games and shortcuts. Some \
                     keyboards only reach full rollover over USB, not Bluetooth."
                } else {
                    ""
                },
            ));
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
        }

        // --- Polling / timing --------------------------------------------
        if self.input_source.has_precise_timestamps() {
            let samples = ctx.polling.intervals_us().len();
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
                                "Inferred from timestamp quantization over {} rapid presses.",
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
                    None => findings.push(Finding::new(
                        Severity::Info,
                        "Polling rate",
                        "No report-interval quantization visible (>= 1000 Hz or interrupt-driven)",
                        format!("{} rapid-press samples analysed.", samples),
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
            let secs = RAPID_TAP_DURATION.as_secs_f64();
            findings.push(Finding::new(
                Severity::Info,
                "Rapid tap",
                format!(
                    "{} presses in {:.0}s ({:.1} per second)",
                    self.rapid_presses,
                    secs,
                    self.rapid_presses as f64 / secs
                ),
                "",
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

        findings.sort_by(|a, b| b.severity.cmp(&a.severity));
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
                let remaining = step.timeout().saturating_sub(self.step_elapsed()).as_secs();
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
                            "{} now, peak {}",
                            self.rollover_pressed.len(),
                            self.rollover_max
                        ),
                    )),
                    StepKind::RapidTap => rows.push(TestResult::info(
                        "Presses",
                        if self.rapid_started.is_some() {
                            format!("{}", self.rapid_presses)
                        } else {
                            "waiting for first tap".to_string()
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
                let status = if issues == 0 {
                    ResultStatus::Ok
                } else {
                    ResultStatus::Error
                };
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

    #[test]
    fn sweep_completes_when_all_expected_keys_seen() {
        let mut t = AutoTest::new(InputSource::Evdev);
        t.set_expected_keys(vec![KeyCode(30), KeyCode(31)]);
        let t0 = Instant::now();
        t.start_at(t0);
        t.tick_at(t0 + IDLE_DURATION); // -> Sweep
        assert_eq!(t.current_step(), Some(StepKind::Sweep));
        t.process_event(&press(KeyCode(30)));
        t.process_event(&release(KeyCode(30)));
        assert!(!t.tick_at(t0 + Duration::from_secs(4)));
        assert_eq!(t.sweep_seen_count(), 1);
        assert_eq!(t.sweep_missing(), vec![KeyCode(31)]);
        t.process_event(&press(KeyCode(31)));
        t.process_event(&release(KeyCode(31)));
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
        t.process_event(&press(KeyCode(30)));
        t.process_event(&release(KeyCode(30)));
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

    #[test]
    fn rollover_step_records_peak_and_completes_after_release() {
        let mut t = AutoTest::new(InputSource::Evdev);
        t.set_expected_keys(vec![]);
        let t0 = Instant::now();
        t.start_at(t0);
        t.tick_at(t0 + IDLE_DURATION);
        t.tick_at(t0 + IDLE_DURATION);
        t.skip_step_at(t0 + IDLE_DURATION); // skip hold
        assert_eq!(t.current_step(), Some(StepKind::Rollover));

        let s = t0 + Duration::from_secs(4);
        for (i, k) in [30u16, 31, 32, 33].iter().enumerate() {
            t.process_event(&evt(
                *k,
                KeyEventType::Press,
                s + Duration::from_millis(i as u64 * 10),
                0,
            ));
        }
        for (i, k) in [30u16, 31, 32, 33].iter().enumerate() {
            t.process_event(&evt(
                *k,
                KeyEventType::Release,
                s + Duration::from_millis(500 + i as u64 * 10),
                0,
            ));
        }
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
        assert_eq!(r.severity, Severity::Warning);
        assert!(r.title.contains("4 keys"));
    }

    #[test]
    fn rapid_tap_detects_chatter_and_completes_sequence() {
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
        let chatter = findings
            .iter()
            .find(|f| f.category == "Key chatter")
            .expect("chatter finding");
        assert_eq!(chatter.severity, Severity::Critical);
        assert!(chatter.detail.contains("A (1x)"));
        // Press-only terminals get the capture limitation warning
        assert!(findings.iter().any(|f| f.category == "Limited capture"));
        // Sorted most severe first
        assert!(findings.windows(2).all(|w| w[0].severity >= w[1].severity));
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
    }

    #[test]
    fn analyze_reports_polling_rate_from_quantized_timestamps() {
        let mut parts = ctx_parts();
        let t0 = Instant::now();
        // 30 presses exactly 8ms*k apart -> 125 Hz grid
        let mut at = t0;
        for i in 0..30u64 {
            let gap = Duration::from_millis(8 * (3 + i % 4)); // 24..56ms
            at += gap;
            parts
                .0
                .process_event(&evt(30, KeyEventType::Press, at, gap.as_micros() as u64));
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
    fn progress_bar_shape() {
        assert_eq!(progress_bar(0.0, 4), "[----]");
        assert_eq!(progress_bar(0.5, 4), "[##--]");
        assert_eq!(progress_bar(1.0, 4), "[####]");
        assert_eq!(progress_bar(7.0, 4), "[####]");
    }
}
