//! Keyboard event types and crossterm-based listener

use super::KeyCode;
use crossterm::event::KeyCode as CtKeyCode;
use std::sync::mpsc;
use std::time::Instant;

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

/// Keyboard listener that converts crossterm key events to KeyEvents.
///
/// Unlike the previous device_query-based listener that polled hardware state,
/// this listener is fed crossterm events from the main loop and translates them
/// into the internal KeyEvent format for test processing.
pub struct KeyboardListener {
    last_poll: Instant,
    event_tx: mpsc::Sender<KeyEvent>,
    /// Whether the event channel is still connected
    channel_alive: bool,
}

impl KeyboardListener {
    /// Create a new keyboard listener
    pub fn new(event_tx: mpsc::Sender<KeyEvent>) -> Self {
        Self {
            last_poll: Instant::now(),
            event_tx,
            channel_alive: true,
        }
    }

    /// Feed a crossterm key press event to generate a KeyEvent.
    /// Returns true if the event was sent successfully.
    pub fn send_press(&mut self, ct_key: CtKeyCode) -> bool {
        self.send_event(ct_key, KeyEventType::Press)
    }

    /// Feed a crossterm key release event to generate a KeyEvent.
    /// Returns true if the event was sent successfully.
    pub fn send_release(&mut self, ct_key: CtKeyCode) -> bool {
        self.send_event(ct_key, KeyEventType::Release)
    }

    fn send_event(&mut self, ct_key: CtKeyCode, event_type: KeyEventType) -> bool {
        if !self.channel_alive {
            return false;
        }

        let now = Instant::now();
        let delta_us = now.duration_since(self.last_poll).as_micros() as u64;
        self.last_poll = now;

        let key = crossterm_to_keycode(ct_key);
        if key.0 == 0 {
            return false; // Unknown key, skip
        }

        let event = KeyEvent::new(key, event_type, now, delta_us);
        if self.event_tx.send(event).is_err() {
            eprintln!("[WARN]  Event channel disconnected, disabling keyboard listener");
            self.channel_alive = false;
            return false;
        }
        true
    }

    /// Get current polling interval in microseconds
    pub fn get_poll_interval_us(&self) -> u64 {
        self.last_poll.elapsed().as_micros() as u64
    }

    /// No-op poll for API compatibility when evdev is unavailable.
    /// Crossterm events are fed via send_press/send_release from the main loop.
    pub fn poll(&mut self) -> usize {
        0
    }
}

/// Convert a crossterm KeyCode to an evdev-compatible KeyCode
fn crossterm_to_keycode(ct: CtKeyCode) -> KeyCode {
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
        // Uppercase letters map to the same scancode (shift is a modifier)
        CtKeyCode::Char(c) if c.is_ascii_uppercase() => {
            return crossterm_to_keycode(CtKeyCode::Char(c.to_ascii_lowercase()));
        }
        _ => 0,
    };
    KeyCode(code)
}
