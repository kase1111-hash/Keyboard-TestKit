//! Keyboard state tracking

use super::{KeyCode, KeyEvent, KeyEventType};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// State of a single key
#[derive(Debug, Clone)]
pub struct KeyState {
    /// Whether the key is currently pressed
    pub is_pressed: bool,
    /// When the key was last pressed
    pub last_press: Option<Instant>,
    /// When the key was last released
    pub last_release: Option<Instant>,
    /// Total number of presses recorded
    pub press_count: u64,
    /// Duration of the last press (press to release)
    pub last_press_duration: Option<Duration>,
    /// Minimum press duration seen
    pub min_press_duration: Option<Duration>,
    /// Maximum press duration seen
    pub max_press_duration: Option<Duration>,
    /// Recent polling intervals for this key (for rate calculation)
    pub recent_intervals_us: Vec<u64>,
}

impl Default for KeyState {
    fn default() -> Self {
        Self {
            is_pressed: false,
            last_press: None,
            last_release: None,
            press_count: 0,
            last_press_duration: None,
            min_press_duration: None,
            max_press_duration: None,
            recent_intervals_us: Vec::with_capacity(100),
        }
    }
}

impl KeyState {
    /// Calculate average polling rate in Hz based on recent intervals
    pub fn avg_polling_rate_hz(&self) -> Option<f64> {
        if self.recent_intervals_us.is_empty() {
            return None;
        }
        let avg_us: f64 = self.recent_intervals_us.iter().sum::<u64>() as f64
            / self.recent_intervals_us.len() as f64;
        if avg_us > 0.0 {
            Some(1_000_000.0 / avg_us)
        } else {
            None
        }
    }

    /// Check if key might be stuck (pressed for too long)
    pub fn is_potentially_stuck(&self, threshold: Duration) -> bool {
        if let (true, Some(press_time)) = (self.is_pressed, self.last_press) {
            press_time.elapsed() > threshold
        } else {
            false
        }
    }

    /// Add a polling interval measurement
    pub fn record_interval(&mut self, interval_us: u64) {
        self.recent_intervals_us.push(interval_us);
        // Keep only last 100 samples
        if self.recent_intervals_us.len() > 100 {
            self.recent_intervals_us.remove(0);
        }
    }
}

/// Overall keyboard state
pub struct KeyboardState {
    /// State for each key
    keys: HashMap<KeyCode, KeyState>,
    /// Currently pressed keys (for rollover counting)
    pressed_keys: Vec<KeyCode>,
    /// Maximum simultaneous keys pressed (NKRO measurement)
    max_simultaneous: usize,
    /// Total events processed
    total_events: u64,
    /// Global polling rate measurements
    global_intervals_us: Vec<u64>,
}

impl KeyboardState {
    pub fn new() -> Self {
        Self {
            keys: HashMap::new(),
            pressed_keys: Vec::new(),
            max_simultaneous: 0,
            total_events: 0,
            global_intervals_us: Vec::with_capacity(1000),
        }
    }

    /// Process a key event and update state
    pub fn process_event(&mut self, event: &KeyEvent) {
        self.total_events += 1;

        // Record global interval
        self.global_intervals_us.push(event.delta_us);
        if self.global_intervals_us.len() > 1000 {
            self.global_intervals_us.remove(0);
        }

        let key_state = self.keys.entry(event.key).or_default();

        match event.event_type {
            KeyEventType::Press => {
                key_state.is_pressed = true;
                key_state.last_press = Some(event.timestamp);
                key_state.press_count += 1;
                key_state.record_interval(event.delta_us);

                // Track pressed keys for rollover
                if !self.pressed_keys.contains(&event.key) {
                    self.pressed_keys.push(event.key);
                }

                // Update max simultaneous
                if self.pressed_keys.len() > self.max_simultaneous {
                    self.max_simultaneous = self.pressed_keys.len();
                }
            }
            KeyEventType::Release => {
                key_state.is_pressed = false;
                key_state.last_release = Some(event.timestamp);

                // Calculate press duration
                if let Some(press_time) = key_state.last_press {
                    let duration = event.timestamp.duration_since(press_time);
                    key_state.last_press_duration = Some(duration);

                    // Update min/max
                    key_state.min_press_duration = Some(
                        key_state.min_press_duration
                            .map(|d| d.min(duration))
                            .unwrap_or(duration)
                    );
                    key_state.max_press_duration = Some(
                        key_state.max_press_duration
                            .map(|d| d.max(duration))
                            .unwrap_or(duration)
                    );
                }

                // Remove from pressed keys
                self.pressed_keys.retain(|k| *k != event.key);
            }
        }
    }

    /// Get state for a specific key
    pub fn get_key_state(&self, key: KeyCode) -> Option<&KeyState> {
        self.keys.get(&key)
    }

    /// Get all currently pressed keys
    pub fn pressed_keys(&self) -> &[KeyCode] {
        &self.pressed_keys
    }

    /// Get current rollover count
    pub fn current_rollover(&self) -> usize {
        self.pressed_keys.len()
    }

    /// Get maximum rollover achieved
    pub fn max_rollover(&self) -> usize {
        self.max_simultaneous
    }

    /// Calculate global average polling rate
    pub fn global_polling_rate_hz(&self) -> Option<f64> {
        if self.global_intervals_us.is_empty() {
            return None;
        }
        let avg_us: f64 = self.global_intervals_us.iter().sum::<u64>() as f64
            / self.global_intervals_us.len() as f64;
        if avg_us > 0.0 {
            Some(1_000_000.0 / avg_us)
        } else {
            None
        }
    }

    /// Get total event count
    pub fn total_events(&self) -> u64 {
        self.total_events
    }

    /// Get all keys that have been used
    pub fn all_keys(&self) -> impl Iterator<Item = (&KeyCode, &KeyState)> {
        self.keys.iter()
    }

    /// Find potentially stuck keys
    pub fn find_stuck_keys(&self, threshold: Duration) -> Vec<KeyCode> {
        self.keys
            .iter()
            .filter(|(_, state)| state.is_potentially_stuck(threshold))
            .map(|(key, _)| *key)
            .collect()
    }

    /// Reset all statistics
    pub fn reset(&mut self) {
        self.keys.clear();
        self.pressed_keys.clear();
        self.max_simultaneous = 0;
        self.total_events = 0;
        self.global_intervals_us.clear();
    }
}

impl Default for KeyboardState {
    fn default() -> Self {
        Self::new()
    }
}
