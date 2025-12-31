//! Shared test utilities for keyboard test modules
//!
//! Provides common helper functions for creating test events and fixtures.

use crate::keyboard::{KeyCode, KeyEvent, KeyEventType};
use std::time::Instant;

/// Default key code used in tests (KeyCode 30 = 'A')
pub const DEFAULT_KEY: KeyCode = KeyCode(30);

/// Default polling interval in microseconds (1000us = 1ms = 1000Hz)
pub const DEFAULT_DELTA_US: u64 = 1000;

/// Creates a key press event with full control over all parameters.
pub fn make_event(
    key: KeyCode,
    event_type: KeyEventType,
    timestamp: Instant,
    delta_us: u64,
) -> KeyEvent {
    KeyEvent {
        key,
        event_type,
        timestamp,
        delta_us,
    }
}

/// Creates a key press event with the specified key and delta.
///
/// Uses `Instant::now()` for the timestamp.
pub fn make_press(key: KeyCode, delta_us: u64) -> KeyEvent {
    KeyEvent {
        key,
        event_type: KeyEventType::Press,
        timestamp: Instant::now(),
        delta_us,
    }
}

/// Creates a key release event with the specified key and delta.
///
/// Uses `Instant::now()` for the timestamp.
pub fn make_release(key: KeyCode, delta_us: u64) -> KeyEvent {
    KeyEvent {
        key,
        event_type: KeyEventType::Release,
        timestamp: Instant::now(),
        delta_us,
    }
}

/// Creates a key press event with default delta (1000us).
pub fn press(key: KeyCode) -> KeyEvent {
    make_press(key, DEFAULT_DELTA_US)
}

/// Creates a key release event with default delta (1000us).
pub fn release(key: KeyCode) -> KeyEvent {
    make_release(key, DEFAULT_DELTA_US)
}

/// Creates a key press event with a specific timestamp and delta.
///
/// Useful for polling rate tests that need precise timing control.
pub fn press_at(key: KeyCode, timestamp: Instant, delta_us: u64) -> KeyEvent {
    make_event(key, KeyEventType::Press, timestamp, delta_us)
}

/// Creates a key release event with a specific timestamp.
pub fn release_at(key: KeyCode, timestamp: Instant) -> KeyEvent {
    make_event(key, KeyEventType::Release, timestamp, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_press_creates_press_event() {
        let event = make_press(KeyCode(30), 500);
        assert_eq!(event.key, KeyCode(30));
        assert_eq!(event.event_type, KeyEventType::Press);
        assert_eq!(event.delta_us, 500);
    }

    #[test]
    fn make_release_creates_release_event() {
        let event = make_release(KeyCode(31), 100);
        assert_eq!(event.key, KeyCode(31));
        assert_eq!(event.event_type, KeyEventType::Release);
        assert_eq!(event.delta_us, 100);
    }

    #[test]
    fn press_uses_default_delta() {
        let event = press(KeyCode(32));
        assert_eq!(event.delta_us, DEFAULT_DELTA_US);
    }

    #[test]
    fn press_at_uses_provided_timestamp() {
        let ts = Instant::now();
        let event = press_at(KeyCode(30), ts, 2000);
        assert_eq!(event.timestamp, ts);
        assert_eq!(event.delta_us, 2000);
    }
}
