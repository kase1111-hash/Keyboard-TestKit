//! Keyboard event types and listener

use super::KeyCode;
use std::time::Instant;
use std::sync::mpsc;
use device_query::{DeviceQuery, DeviceState};

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

/// Keyboard listener that polls for key state changes
pub struct KeyboardListener {
    device_state: DeviceState,
    last_keys: Vec<device_query::Keycode>,
    last_poll: Instant,
    event_tx: mpsc::Sender<KeyEvent>,
}

impl KeyboardListener {
    /// Create a new keyboard listener
    pub fn new(event_tx: mpsc::Sender<KeyEvent>) -> Self {
        Self {
            device_state: DeviceState::new(),
            last_keys: Vec::new(),
            last_poll: Instant::now(),
            event_tx,
        }
    }

    /// Poll for keyboard state changes
    /// Returns the number of events generated
    pub fn poll(&mut self) -> usize {
        let now = Instant::now();
        let delta_us = now.duration_since(self.last_poll).as_micros() as u64;
        self.last_poll = now;

        let current_keys = self.device_state.get_keys();
        let mut event_count = 0;

        // Check for new key presses
        for key in &current_keys {
            if !self.last_keys.contains(key) {
                let event = KeyEvent::new(
                    KeyCode::from(*key),
                    KeyEventType::Press,
                    now,
                    delta_us,
                );
                let _ = self.event_tx.send(event);
                event_count += 1;
            }
        }

        // Check for key releases
        for key in &self.last_keys {
            if !current_keys.contains(key) {
                let event = KeyEvent::new(
                    KeyCode::from(*key),
                    KeyEventType::Release,
                    now,
                    delta_us,
                );
                let _ = self.event_tx.send(event);
                event_count += 1;
            }
        }

        self.last_keys = current_keys;
        event_count
    }

    /// Get current polling interval in microseconds
    pub fn get_poll_interval_us(&self) -> u64 {
        self.last_poll.elapsed().as_micros() as u64
    }
}
