//! Keyboard event types and crossterm-based listener

use super::KeyCode;
use crossterm::event::KeyCode as CtKeyCode;
use std::collections::HashMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Type of keyboard event
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEventType {
    /// Key was pressed down
    Press,
    /// Key was released
    Release,
}

/// A keyboard event with timing information
#[derive(Debug, Clone)]
pub struct KeyEvent {
    /// The key code
    pub key: KeyCode,
    /// Type of event (press/release)
    pub event_type: KeyEventType,
    /// When the event occurred
    pub timestamp: Instant,
    /// Time since last event (for polling rate calculation)
    pub delta_us: u64,
}

impl KeyEvent {
    pub fn new(key: KeyCode, event_type: KeyEventType, timestamp: Instant, delta_us: u64) -> Self {
        Self {
            key,
            event_type,
            timestamp,
            delta_us,
        }
    }
}

/// How long a key stays "pressed" after its last autorepeat report when the
/// terminal cannot report key releases. It is also the longest gap between
/// two reports that still counts as autorepeat. OS autorepeat intervals are
/// typically 30-50 ms (X11 40 ms, Windows ~33 ms, GNOME 30 ms, macOS 30-90 ms),
/// so a held key keeps refreshing this timer.
pub const AUTO_RELEASE_TIMEOUT: Duration = Duration::from_millis(120);

/// Autorepeat practically never starts sooner than this after a key goes
/// down: the shortest delay the Windows and macOS settings offer is 250 ms and
/// 225 ms, and desktop defaults are 500-660 ms. A second report of the same
/// key inside this window is therefore a genuine new press, or chatter.
const MIN_AUTOREPEAT_DELAY: Duration = Duration::from_millis(200);

/// How long a tapped key that never started repeating stays "pressed". It
/// must outlast the autorepeat delay so a held key is not released before its
/// first repeat arrives: 750 ms covers the common defaults (Windows/GNOME
/// 500 ms, KDE 600 ms, X11 660 ms) plus read latency.
const INITIAL_HOLD_TIMEOUT: Duration = Duration::from_millis(750);

/// Longest autorepeat delay common settings offer (Windows 1 s, macOS 1.8 s).
/// A report this soon after a press that has not repeated yet may
/// be that press' first autorepeat even after [`INITIAL_HOLD_TIMEOUT`] released
/// it, so it is only counted once no repeat cadence follows.
const MAX_AUTOREPEAT_DELAY: Duration = Duration::from_secs(2);

/// Two reports of the same key closer together than this did not come from a
/// finger: even drumming on one key with two fingers stays above ~60 ms
/// (16 Hz). They are autorepeat (20 Hz or faster: X11 and KDE 25 Hz, GNOME
/// and Windows ~30 Hz by default) or reports that a stalled read delivered
/// at once. The one exception is the first report after an idle key's press,
/// which autorepeat cannot produce: that is switch chatter.
const MIN_REPRESS_GAP: Duration = Duration::from_millis(50);

/// Per-key bookkeeping of the listener.
#[derive(Debug, Clone, Copy)]
struct KeyTrack {
    /// Whether the tests currently see this key as down
    down: bool,
    /// Timestamp of the last press emitted for this key
    pressed_at: Instant,
    /// Last time the terminal reported this key (press or repeat)
    last_report: Instant,
    /// Autorepeat was seen for the current press
    repeated: bool,
    /// Press-only mode: a report late enough to be the press' first
    /// autorepeat, but possibly a new tap. The next report (repeat cadence or
    /// not) or [`AUTO_RELEASE_TIMEOUT`] of silence decides.
    pending: Option<Instant>,
    /// The key was idle before this press, so no autorepeat can be running:
    /// a report right after it is chatter (see [`MIN_REPRESS_GAP`]).
    idle_before: bool,
    /// The press was a UI command, not test input (see
    /// [`KeyboardListener::swallow_press`]): it, its autorepeat and its
    /// release are not emitted.
    silent: bool,
}

impl KeyTrack {
    fn pressed(now: Instant) -> Self {
        Self {
            down: true,
            pressed_at: now,
            last_report: now,
            repeated: false,
            pending: None,
            idle_before: false,
            silent: false,
        }
    }
}

/// A key event the listener has decided to emit.
type Emit = (Instant, KeyCode, KeyEventType);

/// Midpoint of two instants (`b` not before `a`).
fn midpoint(a: Instant, b: Instant) -> Instant {
    a + b.saturating_duration_since(a) / 2
}

/// Keyboard listener that converts crossterm key events to KeyEvents.
///
/// This listener is fed crossterm events from the main loop and translates
/// them into the internal [`KeyEvent`] format for test processing.
///
/// Most terminals only report key *presses*, and a held key arrives as a
/// stream of identical presses (OS autorepeat). When release reporting is not
/// available (see [`KeyboardListener::set_release_reporting`]) the listener
/// tells the two apart from their timing:
///
/// - a report within 200 ms of the key's press is a new press (autorepeat
///   cannot have started yet), so fast taps and chatter are counted, unless
///   it comes faster than a finger can press again ([`MIN_REPRESS_GAP`]):
///   that is autorepeat, so a held key never turns into a burst of presses;
/// - a later report is held back until the next one shows whether it starts
///   a repeat cadence (gap below [`AUTO_RELEASE_TIMEOUT`]) or was another tap;
/// - releases are synthesized 750 ms after a press that never repeated (longer
///   than common autorepeat delays, so a held key is not cut short), or
///   [`AUTO_RELEASE_TIMEOUT`] after the last autorepeat.
///
/// Synthesized releases are estimates, so hold times and rollover are not
/// measurable in this mode.
pub struct KeyboardListener {
    /// Timestamp of the last event that was emitted (for `delta_us`)
    last_event: Option<Instant>,
    event_tx: mpsc::Sender<KeyEvent>,
    /// Whether the event channel is still connected
    channel_alive: bool,
    /// Whether the terminal reports real key releases (kitty protocol / Windows)
    release_reporting: bool,
    /// Keys seen recently, down or recently released
    keys: HashMap<KeyCode, KeyTrack>,
}

impl KeyboardListener {
    /// Create a new keyboard listener
    pub fn new(event_tx: mpsc::Sender<KeyEvent>) -> Self {
        Self {
            last_event: None,
            event_tx,
            channel_alive: true,
            release_reporting: false,
            keys: HashMap::new(),
        }
    }

    /// Tell the listener whether the terminal reports key releases.
    ///
    /// When `false` (the default), releases are synthesized by
    /// [`KeyboardListener::poll`].
    pub fn set_release_reporting(&mut self, supported: bool) {
        self.release_reporting = supported;
    }

    /// Whether the terminal reports real key releases.
    pub fn release_reporting(&self) -> bool {
        self.release_reporting
    }

    /// Keys currently considered pressed.
    pub fn pressed_keys(&self) -> Vec<KeyCode> {
        self.keys
            .iter()
            .filter(|(_, t)| t.down)
            .map(|(k, _)| *k)
            .collect()
    }

    /// Feed a crossterm key press event to generate a KeyEvent.
    ///
    /// With release reporting, a press for a key that is already down is a
    /// key repeat and generates nothing. Without it, see the type docs.
    /// Returns true if a press event was sent.
    pub fn send_press(&mut self, ct_key: CtKeyCode) -> bool {
        self.press_at(ct_key, Instant::now())
    }

    /// Note a key press that was a UI command rather than test input, such as
    /// the `A` that starts the auto test.
    ///
    /// Nothing is emitted for it, but the key is known to be down, so its
    /// autorepeat and its release (real or synthesized) are not mistaken for
    /// a new keystroke either: a user holding `A` a little too long would
    /// otherwise be told the idle step saw phantom input. The next genuine
    /// press of the key is reported as usual.
    pub fn swallow_press(&mut self, ct_key: CtKeyCode) {
        self.swallow_press_at(ct_key, Instant::now());
    }

    fn swallow_press_at(&mut self, ct_key: CtKeyCode, now: Instant) {
        let key = crossterm_to_keycode(ct_key);
        if key.0 == 0 {
            return;
        }
        self.keys.insert(
            key,
            KeyTrack {
                silent: true,
                ..KeyTrack::pressed(now)
            },
        );
    }

    /// Feed a crossterm key repeat event. Keeps the key alive for auto-release
    /// purposes but never generates an event for a key that is down.
    pub fn send_repeat(&mut self, ct_key: CtKeyCode) {
        let key = crossterm_to_keycode(ct_key);
        if key.0 == 0 {
            return;
        }
        let now = Instant::now();
        if !self.release_reporting {
            self.report_press_only(key, now);
            return;
        }
        match self.keys.get_mut(&key) {
            Some(track) if track.down => track.last_report = now,
            // A repeat for a key we never saw pressed: treat it as a press.
            _ => {
                self.press_at(ct_key, now);
            }
        }
    }

    /// Feed a crossterm key release event to generate a KeyEvent.
    ///
    /// Releases for keys that are not currently pressed are ignored.
    /// Returns true if an event was sent.
    pub fn send_release(&mut self, ct_key: CtKeyCode) -> bool {
        let key = crossterm_to_keycode(ct_key);
        if key.0 == 0 || !self.keys.remove(&key).is_some_and(|t| t.down && !t.silent) {
            return false;
        }
        self.emit(key, KeyEventType::Release, Instant::now())
    }

    fn press_at(&mut self, ct_key: CtKeyCode, now: Instant) -> bool {
        let key = crossterm_to_keycode(ct_key);
        if key.0 == 0 {
            return false; // Unknown key, skip
        }
        if !self.release_reporting {
            return self.report_press_only(key, now);
        }
        if let Some(track) = self.keys.get_mut(&key).filter(|t| t.down) {
            // Already down: this is a terminal key repeat, not a new press.
            track.last_report = now;
            return false;
        }
        self.keys.insert(key, KeyTrack::pressed(now));
        self.emit(key, KeyEventType::Press, now)
    }

    /// Handle a press or repeat report from a terminal that cannot report
    /// releases. Returns true if a press event for `key` was sent.
    fn report_press_only(&mut self, key: KeyCode, now: Instant) -> bool {
        let mut out = Vec::new();
        self.expire_into(now, &mut out);
        // Autorepeat only ever repeats the most recently pressed key, so a
        // report of another key means a pending report was a separate tap.
        for (other, track) in self.keys.iter_mut() {
            if *other != key {
                Self::settle_pending_as_tap(*other, track, &mut out);
            }
        }

        let pressed = match self.keys.get(&key).copied() {
            None => {
                let track = KeyTrack {
                    idle_before: true,
                    ..KeyTrack::pressed(now)
                };
                self.keys.insert(key, track);
                out.push((now, key, KeyEventType::Press));
                true
            }
            Some(mut track) => {
                let pressed = Self::classify_report(key, &mut track, now, &mut out);
                self.keys.insert(key, track);
                pressed
            }
        };
        self.emit_all(out);
        pressed
    }

    /// Decide what a report of a key we already track means. Pushes the
    /// resulting events to `out` and returns true if it is a new press.
    fn classify_report(key: KeyCode, t: &mut KeyTrack, now: Instant, out: &mut Vec<Emit>) -> bool {
        if t.pending.take().is_some() {
            // Two late reports in quick succession: a repeat cadence, so the
            // held-back report was the first autorepeat of a held key.
            t.repeated = true;
            t.last_report = now;
            return false;
        }
        let since_report = now.saturating_duration_since(t.last_report);
        let since_press = now.saturating_duration_since(t.pressed_at);
        if t.silent && !t.repeated && since_press < MIN_AUTOREPEAT_DELAY {
            // A command keystroke is not test input: a report too soon to
            // tell a short-delay autorepeat from a re-press is its repeat.
            t.repeated = true;
            t.last_report = now;
            return false;
        }
        if t.repeated && since_report < AUTO_RELEASE_TIMEOUT {
            // Autorepeat continues (also for a key released early by
            // INITIAL_HOLD_TIMEOUT whose repeat started late).
            t.last_report = now;
            return false;
        }
        if !t.idle_before && since_report < MIN_REPRESS_GAP {
            // Too fast for a finger: autorepeat that started within
            // MIN_AUTOREPEAT_DELAY (a short repeat delay setting), or repeats
            // a stalled read delivered together. Counting it would turn the
            // rest of the hold into a burst of presses at the repeat rate.
            t.repeated = true;
            t.last_report = now;
            return false;
        }
        let may_be_first_repeat = !t.repeated && since_press < MAX_AUTOREPEAT_DELAY;
        if may_be_first_repeat && since_press >= MIN_AUTOREPEAT_DELAY {
            // First autorepeat of a held key, or a new tap: the next report
            // tells (see `settle_pending_as_tap` for the tap case).
            t.pending = Some(now);
            t.last_report = now;
            return false;
        }
        // Too soon for autorepeat, or long after the previous press or its
        // autorepeat ended: a genuine new press. The release in between was
        // not reported; place it halfway so neither the hold nor the gap is
        // invented as zero.
        if t.down && !t.silent {
            out.push((midpoint(t.pressed_at, now), key, KeyEventType::Release));
        }
        *t = KeyTrack::pressed(now);
        out.push((now, key, KeyEventType::Press));
        true
    }

    /// A held-back report turned out not to start a repeat cadence: it was a
    /// separate tap. Emit it with its original timestamp.
    fn settle_pending_as_tap(key: KeyCode, t: &mut KeyTrack, out: &mut Vec<Emit>) {
        let Some(at) = t.pending.take() else {
            return;
        };
        if t.down && !t.silent {
            out.push((midpoint(t.pressed_at, at), key, KeyEventType::Release));
        }
        *t = KeyTrack::pressed(at);
        out.push((at, key, KeyEventType::Press));
    }

    /// Collect the releases (and held-back taps) that are due at `now`.
    fn expire_into(&mut self, now: Instant, out: &mut Vec<Emit>) {
        for (key, t) in self.keys.iter_mut() {
            if t.pending
                .is_some_and(|at| now.saturating_duration_since(at) >= AUTO_RELEASE_TIMEOUT)
            {
                Self::settle_pending_as_tap(*key, t, out);
            }
            if !t.down || t.pending.is_some() {
                continue;
            }
            let deadline = if t.repeated {
                t.last_report + AUTO_RELEASE_TIMEOUT
            } else {
                t.pressed_at + INITIAL_HOLD_TIMEOUT
            };
            if now >= deadline {
                // Stamped at the deadline so the hold time does not depend on
                // how often the main loop polls.
                if !t.silent {
                    out.push((deadline, *key, KeyEventType::Release));
                }
                t.down = false;
            }
        }
        // Forget keys that can no longer affect how the next report is read.
        self.keys.retain(|_, t| {
            t.down
                || t.pending.is_some()
                || now.saturating_duration_since(t.last_report) < MAX_AUTOREPEAT_DELAY
        });
    }

    /// Emit events in timestamp order. Returns how many were sent.
    fn emit_all(&mut self, mut events: Vec<Emit>) -> usize {
        events.sort_by_key(|(ts, _, _)| *ts);
        let mut sent = 0;
        for (timestamp, key, kind) in events {
            if self.emit(key, kind, timestamp) {
                sent += 1;
            }
        }
        sent
    }

    fn emit(&mut self, key: KeyCode, event_type: KeyEventType, timestamp: Instant) -> bool {
        if !self.channel_alive {
            return false;
        }
        let delta_us = self
            .last_event
            .map(|last| timestamp.saturating_duration_since(last).as_micros() as u64)
            .unwrap_or(0);
        self.last_event = Some(
            self.last_event
                .map_or(timestamp, |last| last.max(timestamp)),
        );

        let event = KeyEvent::new(key, event_type, timestamp, delta_us);
        if self.event_tx.send(event).is_err() {
            eprintln!("[WARN]  Event channel disconnected, disabling keyboard listener");
            self.channel_alive = false;
            return false;
        }
        true
    }

    /// Get time since the last emitted event in microseconds
    pub fn get_poll_interval_us(&self) -> u64 {
        self.last_event
            .map(|t| t.elapsed().as_micros() as u64)
            .unwrap_or(0)
    }

    /// Synthesize releases for keys the terminal never reported as released,
    /// and emit held-back taps once it is clear no autorepeat followed them.
    ///
    /// Does nothing when the terminal reports releases itself. Returns the
    /// number of events generated.
    pub fn poll(&mut self) -> usize {
        self.poll_at(Instant::now())
    }

    fn poll_at(&mut self, now: Instant) -> usize {
        if self.release_reporting {
            return 0;
        }
        let mut out = Vec::new();
        self.expire_into(now, &mut out);
        self.emit_all(out)
    }

    /// Forget all pressed keys without emitting events (e.g. after a reset).
    pub fn reset(&mut self) {
        self.keys.clear();
        self.last_event = None;
    }
}

/// Convert a crossterm KeyCode to an evdev-compatible KeyCode
pub fn crossterm_to_keycode(ct: CtKeyCode) -> KeyCode {
    use crossterm::event::ModifierKeyCode as Mk;
    let code = match ct {
        CtKeyCode::Esc => 1,
        CtKeyCode::Char('1') => 2,
        CtKeyCode::Char('2') => 3,
        CtKeyCode::Char('3') => 4,
        CtKeyCode::Char('4') => 5,
        CtKeyCode::Char('5') => 6,
        CtKeyCode::Char('6') => 7,
        CtKeyCode::Char('7') => 8,
        CtKeyCode::Char('8') => 9,
        CtKeyCode::Char('9') => 10,
        CtKeyCode::Char('0') => 11,
        CtKeyCode::Char('-') => 12,
        CtKeyCode::Char('=') => 13,
        CtKeyCode::Backspace => 14,
        CtKeyCode::Tab | CtKeyCode::BackTab => 15,
        CtKeyCode::Char('q') => 16,
        CtKeyCode::Char('w') => 17,
        CtKeyCode::Char('e') => 18,
        CtKeyCode::Char('r') => 19,
        CtKeyCode::Char('t') => 20,
        CtKeyCode::Char('y') => 21,
        CtKeyCode::Char('u') => 22,
        CtKeyCode::Char('i') => 23,
        CtKeyCode::Char('o') => 24,
        CtKeyCode::Char('p') => 25,
        CtKeyCode::Char('[') => 26,
        CtKeyCode::Char(']') => 27,
        CtKeyCode::Enter => 28,
        CtKeyCode::Char('a') => 30,
        CtKeyCode::Char('s') => 31,
        CtKeyCode::Char('d') => 32,
        CtKeyCode::Char('f') => 33,
        CtKeyCode::Char('g') => 34,
        CtKeyCode::Char('h') => 35,
        CtKeyCode::Char('j') => 36,
        CtKeyCode::Char('k') => 37,
        CtKeyCode::Char('l') => 38,
        CtKeyCode::Char(';') => 39,
        CtKeyCode::Char('\'') => 40,
        CtKeyCode::Char('`') => 41,
        CtKeyCode::Char('\\') => 43,
        CtKeyCode::Char('z') => 44,
        CtKeyCode::Char('x') => 45,
        CtKeyCode::Char('c') => 46,
        CtKeyCode::Char('v') => 47,
        CtKeyCode::Char('b') => 48,
        CtKeyCode::Char('n') => 49,
        CtKeyCode::Char('m') => 50,
        CtKeyCode::Char(',') => 51,
        CtKeyCode::Char('.') => 52,
        CtKeyCode::Char('/') => 53,
        CtKeyCode::Char(' ') => 57,
        CtKeyCode::CapsLock => 58,
        CtKeyCode::F(1) => 59,
        CtKeyCode::F(2) => 60,
        CtKeyCode::F(3) => 61,
        CtKeyCode::F(4) => 62,
        CtKeyCode::F(5) => 63,
        CtKeyCode::F(6) => 64,
        CtKeyCode::F(7) => 65,
        CtKeyCode::F(8) => 66,
        CtKeyCode::F(9) => 67,
        CtKeyCode::F(10) => 68,
        CtKeyCode::F(11) => 87,
        CtKeyCode::F(12) => 88,
        CtKeyCode::ScrollLock => 70,
        CtKeyCode::Pause => 119,
        CtKeyCode::Insert => 110,
        CtKeyCode::Home => 102,
        CtKeyCode::PageUp => 104,
        CtKeyCode::Delete => 111,
        CtKeyCode::End => 107,
        CtKeyCode::PageDown => 109,
        CtKeyCode::Up => 103,
        CtKeyCode::Left => 105,
        CtKeyCode::Down => 108,
        CtKeyCode::Right => 106,
        CtKeyCode::NumLock => 69,
        CtKeyCode::PrintScreen => 99,
        CtKeyCode::Menu => 127,
        CtKeyCode::KeypadBegin => 76,
        // Modifier keys are only reported by terminals with the kitty
        // keyboard protocol (REPORT_ALL_KEYS_AS_ESCAPE_CODES) enabled.
        CtKeyCode::Modifier(Mk::LeftShift) => 42,
        CtKeyCode::Modifier(Mk::RightShift) => 54,
        CtKeyCode::Modifier(Mk::LeftControl) => 29,
        CtKeyCode::Modifier(Mk::RightControl) => 97,
        CtKeyCode::Modifier(Mk::LeftAlt) => 56,
        CtKeyCode::Modifier(Mk::RightAlt | Mk::IsoLevel3Shift) => 100,
        CtKeyCode::Modifier(Mk::LeftSuper | Mk::LeftMeta | Mk::LeftHyper) => 125,
        CtKeyCode::Modifier(Mk::RightSuper | Mk::RightMeta | Mk::RightHyper) => 126,
        // Uppercase letters map to the same scancode (shift is a modifier)
        CtKeyCode::Char(c) if c.is_ascii_uppercase() => {
            return crossterm_to_keycode(CtKeyCode::Char(c.to_ascii_lowercase()));
        }
        // Shifted symbols map to the physical key that produces them (US layout)
        CtKeyCode::Char(c) => match unshift_symbol(c) {
            Some(base) => return crossterm_to_keycode(CtKeyCode::Char(base)),
            None => 0,
        },
        _ => 0,
    };
    KeyCode(code)
}

/// Map a shifted US-layout symbol back to the unshifted character on the same key.
fn unshift_symbol(c: char) -> Option<char> {
    Some(match c {
        '!' => '1',
        '@' => '2',
        '#' => '3',
        '$' => '4',
        '%' => '5',
        '^' => '6',
        '&' => '7',
        '*' => '8',
        '(' => '9',
        ')' => '0',
        '_' => '-',
        '+' => '=',
        '{' => '[',
        '}' => ']',
        '|' => '\\',
        ':' => ';',
        '"' => '\'',
        '<' => ',',
        '>' => '.',
        '?' => '/',
        '~' => '`',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listener() -> (KeyboardListener, mpsc::Receiver<KeyEvent>) {
        let (tx, rx) = mpsc::channel();
        (KeyboardListener::new(tx), rx)
    }

    #[test]
    fn press_then_release_emits_two_events() {
        let (mut l, rx) = listener();
        l.set_release_reporting(true);
        assert!(l.send_press(CtKeyCode::Char('a')));
        assert!(l.send_release(CtKeyCode::Char('a')));
        let events: Vec<KeyEvent> = rx.try_iter().collect();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].key, KeyCode(30));
        assert_eq!(events[0].event_type, KeyEventType::Press);
        assert_eq!(events[1].event_type, KeyEventType::Release);
        assert!(l.pressed_keys().is_empty());
    }

    #[test]
    fn repeated_press_is_deduplicated() {
        let (mut l, rx) = listener();
        l.set_release_reporting(true);
        assert!(l.send_press(CtKeyCode::Char('a')));
        assert!(!l.send_press(CtKeyCode::Char('a')));
        assert!(!l.send_press(CtKeyCode::Char('a')));
        assert_eq!(rx.try_iter().count(), 1);
    }

    #[test]
    fn release_without_press_is_ignored() {
        let (mut l, rx) = listener();
        l.set_release_reporting(true);
        assert!(!l.send_release(CtKeyCode::Char('a')));
        assert_eq!(rx.try_iter().count(), 0);
    }

    #[test]
    fn auto_release_synthesized_when_terminal_cannot_report_releases() {
        let (mut l, rx) = listener();
        let t0 = Instant::now();
        assert!(l.press_at(CtKeyCode::Char('a'), t0));
        // A tap stays down until autorepeat could have started
        assert_eq!(l.poll_at(t0 + Duration::from_millis(500)), 0);
        assert_eq!(l.pressed_keys(), vec![KeyCode(30)]);
        // Expired
        assert_eq!(l.poll_at(t0 + INITIAL_HOLD_TIMEOUT), 1);
        assert!(l.pressed_keys().is_empty());
        let events: Vec<KeyEvent> = rx.try_iter().collect();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].event_type, KeyEventType::Release);
        assert_eq!(events[1].timestamp, t0 + INITIAL_HOLD_TIMEOUT);
    }

    #[test]
    fn repeat_keeps_key_alive() {
        let (mut l, _rx) = listener();
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        assert!(l.press_at(CtKeyCode::Char('a'), t0));
        // First autorepeat after a 500 ms delay, then every 33 ms
        assert!(!l.press_at(CtKeyCode::Char('a'), ms(500)));
        assert!(!l.press_at(CtKeyCode::Char('a'), ms(533)));
        assert!(!l.press_at(CtKeyCode::Char('a'), ms(566)));
        assert_eq!(l.poll_at(ms(600)), 0);
        assert_eq!(l.pressed_keys(), vec![KeyCode(30)]);
        assert_eq!(l.poll_at(ms(566) + AUTO_RELEASE_TIMEOUT), 1);
        assert!(l.pressed_keys().is_empty());
    }

    /// Drive a press-only listener like the main loop does: poll every
    /// 16 ms frame, deliver each report (ms after start) at its time.
    fn run_press_only(reports: &[(u64, char)], end_ms: u64) -> Vec<KeyEvent> {
        run_with_stalls(reports, end_ms, &[])
    }

    /// Like [`run_press_only`], but the main loop is stuck (slow redraw,
    /// loaded machine, laggy SSH link) during each `(from_ms, to_ms)` stall:
    /// nothing is polled, and the reports made meanwhile are all read when it
    /// ends, with the same read time.
    fn run_with_stalls(
        reports: &[(u64, char)],
        end_ms: u64,
        stalls: &[(u64, u64)],
    ) -> Vec<KeyEvent> {
        let (mut l, rx) = listener();
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let stall_end = |ms: u64| {
            stalls
                .iter()
                .find(|(from, to)| (*from..*to).contains(&ms))
                .map(|(_, to)| *to)
        };
        let mut next = 0;
        let mut frame = 0;
        while frame <= end_ms {
            if stall_end(frame).is_none() {
                l.poll_at(at(frame));
            }
            while next < reports.len() && reports[next].0 < frame + 16 {
                let (ms, c) = reports[next];
                l.press_at(CtKeyCode::Char(c), at(stall_end(ms).unwrap_or(ms)));
                next += 1;
            }
            frame += 16;
        }
        rx.try_iter().collect()
    }

    /// Shortest gap between two consecutive presses.
    fn min_press_gap(events: &[KeyEvent]) -> Option<Duration> {
        let presses: Vec<Instant> = events
            .iter()
            .filter(|e| e.event_type == KeyEventType::Press)
            .map(|e| e.timestamp)
            .collect();
        presses
            .windows(2)
            .map(|w| w[1].saturating_duration_since(w[0]))
            .min()
    }

    fn presses(events: &[KeyEvent]) -> usize {
        events
            .iter()
            .filter(|e| e.event_type == KeyEventType::Press)
            .count()
    }

    /// A key held down: initial press, autorepeat after `delay_ms`, then a
    /// report every `interval_ms` until `until_ms`.
    fn hold(c: char, delay_ms: u64, interval_ms: u64, until_ms: u64) -> Vec<(u64, char)> {
        let mut reports = vec![(0, c)];
        let mut t = delay_ms;
        while t <= until_ms {
            reports.push((t, c));
            t += interval_ms;
        }
        reports
    }

    #[test]
    fn fast_repress_in_press_only_mode_is_a_new_press() {
        // Tapping at ~11 Hz: each tap is a new press, not an autorepeat
        let taps: Vec<(u64, char)> = (0..20).map(|i| (i * 90, 'a')).collect();
        let events = run_press_only(&taps, 3000);
        assert_eq!(presses(&events), 20);
        // Every press except the last is followed by a synthesized release
        // placed halfway to the next tap, so no bounce is invented.
        let first_release = events
            .iter()
            .find(|e| e.event_type == KeyEventType::Release)
            .unwrap();
        assert_eq!(
            first_release
                .timestamp
                .duration_since(events[0].timestamp)
                .as_millis(),
            45
        );
    }

    #[test]
    fn chatter_double_press_is_reported() {
        // A worn switch registering one press twice, 12 ms apart
        let events = run_press_only(&[(0, 'a'), (12, 'a')], 1500);
        assert_eq!(presses(&events), 2);
    }

    #[test]
    fn held_key_is_one_press_and_stays_down_until_repeat() {
        for (delay, interval) in [(250, 30), (500, 33), (660, 40), (375, 90)] {
            let (mut l, rx) = listener();
            let t0 = Instant::now();
            assert!(l.press_at(CtKeyCode::Char('a'), t0));
            // Not cut short before the first autorepeat arrives
            assert_eq!(l.poll_at(t0 + Duration::from_millis(delay - 10)), 0);
            assert_eq!(l.pressed_keys(), vec![KeyCode(30)]);
            drop(rx);

            let events = run_press_only(&hold('a', delay, interval, 2000), 2500);
            assert_eq!(
                presses(&events),
                1,
                "delay {delay} ms, interval {interval} ms"
            );
            let releases: Vec<&KeyEvent> = events
                .iter()
                .filter(|e| e.event_type == KeyEventType::Release)
                .collect();
            assert_eq!(releases.len(), 1);
            // Released shortly after the last autorepeat
            let held = releases[0].timestamp.duration_since(events[0].timestamp);
            assert!(held >= Duration::from_millis(2000 - interval), "{held:?}");
            assert!(held <= Duration::from_millis(2000) + AUTO_RELEASE_TIMEOUT);
        }
    }

    #[test]
    fn long_autorepeat_delay_does_not_turn_repeats_into_presses() {
        // 1 s delay (Windows "long" setting): the key is released early, but
        // the repeat stream must not be read as a burst of new presses.
        let events = run_press_only(&hold('a', 1000, 33, 2500), 3000);
        assert_eq!(presses(&events), 1);
    }

    #[test]
    fn slow_taps_are_all_counted() {
        for interval in [210, 300, 450, 700, 1000, 2500] {
            let taps: Vec<(u64, char)> = (0..8).map(|i| (i * interval, 'a')).collect();
            let events = run_press_only(&taps, 8 * interval + 1000);
            assert_eq!(presses(&events), 8, "tapping every {interval} ms");
            // Delivered in time order
            assert!(events.windows(2).all(|w| w[0].timestamp <= w[1].timestamp));
        }
    }

    #[test]
    fn typing_other_keys_settles_a_held_back_report() {
        // "a", "a" 300 ms later (held back: could be autorepeat), then "s":
        // autorepeat would have switched to "s", so the second "a" was a tap
        // and is emitted before the "s" press.
        let (mut l, rx) = listener();
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        assert!(l.press_at(CtKeyCode::Char('a'), t0));
        assert!(!l.press_at(CtKeyCode::Char('a'), ms(300)));
        assert!(l.press_at(CtKeyCode::Char('s'), ms(340)));
        let events: Vec<KeyEvent> = rx.try_iter().collect();
        let summary: Vec<(u16, KeyEventType)> =
            events.iter().map(|e| (e.key.0, e.event_type)).collect();
        assert_eq!(
            summary,
            vec![
                (30, KeyEventType::Press),
                (30, KeyEventType::Release),
                (30, KeyEventType::Press),
                (31, KeyEventType::Press),
            ]
        );
        assert_eq!(events[2].timestamp, ms(300));
    }

    #[test]
    fn repress_after_autorepeat_ends_is_a_new_press() {
        let mut reports = hold('a', 500, 33, 900);
        reports.push((1200, 'a'));
        let events = run_press_only(&reports, 2500);
        assert_eq!(presses(&events), 2);
    }

    #[test]
    fn short_autorepeat_delay_does_not_turn_a_hold_into_a_burst() {
        // Repeat delays below 200 ms (xset r rate 150-200, macOS
        // InitialKeyRepeat 10-13, or 200 ms read slightly early): the first
        // autorepeat looks like a fast re-press, but the repeats after it
        // come faster than a finger and must not each count as a press.
        for (delay, interval) in [(150, 15), (150, 30), (180, 20), (195, 33), (199, 40)] {
            let events = run_press_only(&hold('a', delay, interval, 1500), 2000);
            assert_eq!(
                presses(&events),
                2,
                "delay {delay} ms, interval {interval} ms"
            );
            assert!(min_press_gap(&events).unwrap() >= Duration::from_millis(delay));
        }
    }

    #[test]
    fn stalled_reads_during_a_hold_are_not_chatter() {
        // Autorepeat queued while the main loop was stuck is read in one go.
        // Same-key presses microseconds apart would be reported as chatter.
        for stall in [(700, 900), (510, 650), (1200, 1500)] {
            let events = run_with_stalls(&hold('a', 500, 33, 2000), 2500, &[stall]);
            assert!(presses(&events) <= 3, "stall {stall:?}: {events:?}");
            assert!(
                min_press_gap(&events).unwrap() >= MIN_REPRESS_GAP,
                "stall {stall:?}"
            );
        }
    }

    #[test]
    fn chatter_on_an_idle_key_is_still_reported_when_read_together() {
        // The doubled report of a chattering switch, read in the same batch
        let events = run_with_stalls(&[(0, 'a'), (6, 'a')], 1500, &[(0, 10)]);
        assert_eq!(presses(&events), 2);
    }

    /// Like [`run_press_only`], but the first report is a command keystroke
    /// handed to [`KeyboardListener::swallow_press`].
    fn run_swallowing_first(reports: &[(u64, char)], end_ms: u64) -> Vec<KeyEvent> {
        let (mut l, rx) = listener();
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut next = 0;
        let mut frame = 0;
        while frame <= end_ms {
            l.poll_at(at(frame));
            while next < reports.len() && reports[next].0 < frame + 16 {
                let (ms, c) = reports[next];
                if next == 0 {
                    l.swallow_press_at(CtKeyCode::Char(c), at(ms));
                } else {
                    l.press_at(CtKeyCode::Char(c), at(ms));
                }
                next += 1;
            }
            frame += 16;
        }
        rx.try_iter().collect()
    }

    #[test]
    fn held_command_key_emits_nothing_in_press_only_mode() {
        // The 'A' that starts the auto test, held past the autorepeat delay:
        // neither its repeats nor its synthesized release are test input
        for (delay, interval) in [(150, 15), (199, 40), (250, 30), (500, 33), (1000, 33)] {
            let events = run_swallowing_first(&hold('A', delay, interval, 1800), 2500);
            assert!(events.is_empty(), "delay {delay} ms: {events:?}");
        }
        // A short tap emits nothing either
        assert!(run_swallowing_first(&[(0, 'A')], 2000).is_empty());
    }

    #[test]
    fn later_presses_of_a_swallowed_key_are_reported() {
        // Pressing A again later (the sweep asks for every key) is real input,
        // after a held command key and after a tapped one
        let mut reports = hold('A', 500, 33, 1200);
        reports.push((4000, 'a'));
        let events = run_swallowing_first(&reports, 5000);
        assert_eq!(presses(&events), 1, "{events:?}");
        assert_eq!(events[0].key, KeyCode(30));

        // A second tap 300 ms after the command is a new keystroke too
        let events = run_swallowing_first(&[(0, 'A'), (300, 'a')], 2000);
        assert_eq!(presses(&events), 1, "{events:?}");
        assert!(!events
            .iter()
            .any(|e| e.event_type == KeyEventType::Release && e.timestamp < events[0].timestamp));
    }

    #[test]
    fn held_command_key_emits_nothing_when_releases_are_reported() {
        let (mut l, rx) = listener();
        l.set_release_reporting(true);
        l.swallow_press(CtKeyCode::Char('a'));
        // Kitty repeat events, Windows console repeats (as presses), release
        for _ in 0..5 {
            l.send_repeat(CtKeyCode::Char('a'));
            assert!(!l.send_press(CtKeyCode::Char('A')));
        }
        assert!(!l.send_release(CtKeyCode::Char('a')));
        assert_eq!(rx.try_iter().count(), 0);
        // The next keystroke of the key is reported normally
        assert!(l.send_press(CtKeyCode::Char('a')));
        assert!(l.send_release(CtKeyCode::Char('a')));
        assert_eq!(rx.try_iter().count(), 2);
    }

    #[test]
    fn no_auto_release_when_terminal_reports_releases() {
        let (mut l, _rx) = listener();
        l.set_release_reporting(true);
        let t0 = Instant::now();
        assert!(l.press_at(CtKeyCode::Char('a'), t0));
        assert_eq!(l.poll_at(t0 + Duration::from_secs(10)), 0);
        assert_eq!(l.pressed_keys(), vec![KeyCode(30)]);
    }

    #[test]
    fn delta_us_measures_time_between_events() {
        let (mut l, rx) = listener();
        l.set_release_reporting(true);
        let t0 = Instant::now();
        l.press_at(CtKeyCode::Char('a'), t0);
        l.press_at(CtKeyCode::Char('s'), t0 + Duration::from_millis(8));
        let events: Vec<KeyEvent> = rx.try_iter().collect();
        assert_eq!(events[0].delta_us, 0);
        assert_eq!(events[1].delta_us, 8000);
    }

    #[test]
    fn shifted_symbols_map_to_physical_key() {
        assert_eq!(crossterm_to_keycode(CtKeyCode::Char('!')), KeyCode(2));
        assert_eq!(crossterm_to_keycode(CtKeyCode::Char(')')), KeyCode(11));
        assert_eq!(crossterm_to_keycode(CtKeyCode::Char('?')), KeyCode(53));
        assert_eq!(crossterm_to_keycode(CtKeyCode::Char('~')), KeyCode(41));
        assert_eq!(crossterm_to_keycode(CtKeyCode::Char('"')), KeyCode(40));
        assert_eq!(crossterm_to_keycode(CtKeyCode::Char('A')), KeyCode(30));
    }

    #[test]
    fn modifier_keys_map_to_scancodes() {
        use crossterm::event::ModifierKeyCode as Mk;
        assert_eq!(
            crossterm_to_keycode(CtKeyCode::Modifier(Mk::LeftShift)),
            KeyCode(42)
        );
        assert_eq!(
            crossterm_to_keycode(CtKeyCode::Modifier(Mk::RightControl)),
            KeyCode(97)
        );
        assert_eq!(
            crossterm_to_keycode(CtKeyCode::Modifier(Mk::LeftSuper)),
            KeyCode(125)
        );
    }

    #[test]
    fn unknown_keys_are_dropped() {
        let (mut l, rx) = listener();
        assert!(!l.send_press(CtKeyCode::Null));
        assert!(!l.send_press(CtKeyCode::Char('é')));
        assert_eq!(rx.try_iter().count(), 0);
    }
}
