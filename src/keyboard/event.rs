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

/// How long a key stays "pressed" after its last press/repeat report when the
/// terminal cannot report key releases. Terminal key-repeat intervals are
/// typically 25-50 ms, so a held key keeps refreshing this timer while the
/// terminal keeps repeating it.
pub const AUTO_RELEASE_TIMEOUT: Duration = Duration::from_millis(120);

/// Keyboard listener that converts crossterm key events to KeyEvents.
///
/// This listener is fed crossterm events from the main loop and translates
/// them into the internal [`KeyEvent`] format for test processing.
///
/// Most terminals only report key *presses*. When release reporting is not
/// available (see [`KeyboardListener::set_release_reporting`]), the listener
/// synthesizes a release for every key that has not been reported again within
/// [`AUTO_RELEASE_TIMEOUT`], so keys never appear permanently stuck and
/// rollover counts stay meaningful.
pub struct KeyboardListener {
    /// Timestamp of the last event that was emitted (for `delta_us`)
    last_event: Option<Instant>,
    event_tx: mpsc::Sender<KeyEvent>,
    /// Whether the event channel is still connected
    channel_alive: bool,
    /// Whether the terminal reports real key releases (kitty protocol / Windows)
    release_reporting: bool,
    /// Keys currently considered pressed, with the time they were last reported
    pressed: HashMap<KeyCode, Instant>,
}

impl KeyboardListener {
    /// Create a new keyboard listener
    pub fn new(event_tx: mpsc::Sender<KeyEvent>) -> Self {
        Self {
            last_event: None,
            event_tx,
            channel_alive: true,
            release_reporting: false,
            pressed: HashMap::new(),
        }
    }

    /// Tell the listener whether the terminal reports key releases.
    ///
    /// When `false` (the default), releases are synthesized after
    /// [`AUTO_RELEASE_TIMEOUT`] by [`KeyboardListener::poll`].
    pub fn set_release_reporting(&mut self, supported: bool) {
        self.release_reporting = supported;
    }

    /// Whether the terminal reports real key releases.
    pub fn release_reporting(&self) -> bool {
        self.release_reporting
    }

    /// Keys currently considered pressed.
    pub fn pressed_keys(&self) -> Vec<KeyCode> {
        self.pressed.keys().copied().collect()
    }

    /// Feed a crossterm key press event to generate a KeyEvent.
    ///
    /// A press for a key that is already down is treated as a key repeat and
    /// does not generate a new event. Returns true if an event was sent.
    pub fn send_press(&mut self, ct_key: CtKeyCode) -> bool {
        self.press_at(ct_key, Instant::now())
    }

    /// Feed a crossterm key repeat event. Keeps the key alive for auto-release
    /// purposes but never generates an event.
    pub fn send_repeat(&mut self, ct_key: CtKeyCode) {
        let key = crossterm_to_keycode(ct_key);
        if key.0 == 0 {
            return;
        }
        let now = Instant::now();
        match self.pressed.get_mut(&key) {
            Some(last_seen) => *last_seen = now,
            // A repeat for a key we never saw pressed: treat it as a press.
            None => {
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
        if key.0 == 0 || self.pressed.remove(&key).is_none() {
            return false;
        }
        self.emit(key, KeyEventType::Release, Instant::now())
    }

    fn press_at(&mut self, ct_key: CtKeyCode, now: Instant) -> bool {
        let key = crossterm_to_keycode(ct_key);
        if key.0 == 0 {
            return false; // Unknown key, skip
        }
        if let Some(last_seen) = self.pressed.get_mut(&key) {
            // Already down: this is a terminal key repeat, not a new press.
            *last_seen = now;
            return false;
        }
        self.pressed.insert(key, now);
        self.emit(key, KeyEventType::Press, now)
    }

    fn emit(&mut self, key: KeyCode, event_type: KeyEventType, now: Instant) -> bool {
        if !self.channel_alive {
            return false;
        }
        let delta_us = self
            .last_event
            .map(|last| now.saturating_duration_since(last).as_micros() as u64)
            .unwrap_or(0);
        self.last_event = Some(now);

        let event = KeyEvent::new(key, event_type, now, delta_us);
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

    /// Synthesize releases for keys the terminal never reported as released.
    ///
    /// Does nothing when the terminal reports releases itself. Returns the
    /// number of release events generated.
    pub fn poll(&mut self) -> usize {
        self.poll_at(Instant::now())
    }

    fn poll_at(&mut self, now: Instant) -> usize {
        if self.release_reporting {
            return 0;
        }
        let expired: Vec<KeyCode> = self
            .pressed
            .iter()
            .filter(|(_, last_seen)| {
                now.saturating_duration_since(**last_seen) >= AUTO_RELEASE_TIMEOUT
            })
            .map(|(key, _)| *key)
            .collect();
        let mut count = 0;
        for key in expired {
            self.pressed.remove(&key);
            if self.emit(key, KeyEventType::Release, now) {
                count += 1;
            }
        }
        count
    }

    /// Forget all pressed keys without emitting events (e.g. after a reset).
    pub fn reset(&mut self) {
        self.pressed.clear();
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
        // Not expired yet
        assert_eq!(l.poll_at(t0 + Duration::from_millis(50)), 0);
        assert_eq!(l.pressed_keys(), vec![KeyCode(30)]);
        // Expired
        assert_eq!(l.poll_at(t0 + AUTO_RELEASE_TIMEOUT), 1);
        assert!(l.pressed_keys().is_empty());
        let events: Vec<KeyEvent> = rx.try_iter().collect();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].event_type, KeyEventType::Release);
    }

    #[test]
    fn repeat_keeps_key_alive() {
        let (mut l, _rx) = listener();
        let t0 = Instant::now();
        assert!(l.press_at(CtKeyCode::Char('a'), t0));
        // Simulate terminal key repeat arriving as another press just before expiry
        assert!(!l.press_at(CtKeyCode::Char('a'), t0 + Duration::from_millis(100)));
        assert_eq!(l.poll_at(t0 + Duration::from_millis(150)), 0);
        assert_eq!(l.pressed_keys(), vec![KeyCode(30)]);
        assert_eq!(
            l.poll_at(t0 + Duration::from_millis(100) + AUTO_RELEASE_TIMEOUT),
            1
        );
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
