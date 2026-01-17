//! Raw evdev-based keyboard listener for Linux
//!
//! This module provides raw scancode detection via evdev, which can detect
//! OEM keys and other special keys that device_query cannot handle.

use super::{KeyCode, KeyEvent, KeyEventType};
use nix::libc;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Read};
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Instant;

/// Error type for evdev operations
#[derive(Debug)]
pub enum EvdevError {
    /// No keyboard devices found
    NoDevices,
    /// Permission denied accessing device
    PermissionDenied(String),
    /// IO error
    Io(io::Error),
    /// Device enumeration failed
    EnumerationFailed(String),
}

impl std::fmt::Display for EvdevError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvdevError::NoDevices => write!(f, "No keyboard devices found"),
            EvdevError::PermissionDenied(path) => {
                write!(f, "Permission denied accessing {}", path)
            }
            EvdevError::Io(e) => write!(f, "IO error: {}", e),
            EvdevError::EnumerationFailed(msg) => write!(f, "Device enumeration failed: {}", msg),
        }
    }
}

impl std::error::Error for EvdevError {}

impl From<io::Error> for EvdevError {
    fn from(e: io::Error) -> Self {
        if e.kind() == io::ErrorKind::PermissionDenied {
            EvdevError::PermissionDenied("device".to_string())
        } else {
            EvdevError::Io(e)
        }
    }
}

/// A raw input event from the kernel
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct InputEvent {
    tv_sec: i64,
    tv_usec: i64,
    event_type: u16,
    code: u16,
    value: i32,
}

const EV_KEY: u16 = 0x01;
const INPUT_EVENT_SIZE: usize = std::mem::size_of::<InputEvent>();

/// Find all keyboard input devices
fn find_keyboard_devices() -> Result<Vec<PathBuf>, EvdevError> {
    let input_dir = PathBuf::from("/dev/input");
    if !input_dir.exists() {
        return Err(EvdevError::EnumerationFailed(
            "/dev/input does not exist".to_string(),
        ));
    }

    let mut keyboards = Vec::new();

    // Try evdev devices first
    if let Ok(entries) = fs::read_dir(&input_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

            // Look for event* devices
            if name.starts_with("event") {
                // Check if this is a keyboard by reading its capabilities
                if is_keyboard_device(&path) {
                    keyboards.push(path);
                }
            }
        }
    }

    if keyboards.is_empty() {
        return Err(EvdevError::NoDevices);
    }

    Ok(keyboards)
}

/// Check if a device is a keyboard by examining /sys/class/input
fn is_keyboard_device(device_path: &PathBuf) -> bool {
    let device_name = device_path.file_name().and_then(|n| n.to_str());
    if let Some(name) = device_name {
        // Try to read device capabilities from sysfs
        let caps_path = format!("/sys/class/input/{}/device/capabilities/key", name);
        if let Ok(caps) = fs::read_to_string(&caps_path) {
            // A keyboard should have the alphabetic keys (scancodes 16-50 roughly)
            // The capabilities are hex bitmaps showing which keys are supported
            // If it has reasonable key capabilities, consider it a keyboard
            let trimmed = caps.trim();
            if !trimmed.is_empty() && trimmed != "0" {
                // Parse the hex capabilities - keyboards typically have many keys
                let total_bits: u32 = trimmed
                    .split_whitespace()
                    .filter_map(|hex| u64::from_str_radix(hex, 16).ok())
                    .map(|n| n.count_ones())
                    .sum();
                // A typical keyboard has 80+ keys mapped
                return total_bits > 50;
            }
        }

        // Fallback: check device name in /sys
        let name_path = format!("/sys/class/input/{}/device/name", name);
        if let Ok(dev_name) = fs::read_to_string(&name_path) {
            let dev_name_lower = dev_name.to_lowercase();
            return dev_name_lower.contains("keyboard")
                || dev_name_lower.contains("kbd")
                || dev_name_lower.contains("hid");
        }
    }
    false
}

/// Evdev-based keyboard listener for raw scancode detection
pub struct EvdevListener {
    devices: Vec<File>,
    device_paths: Vec<PathBuf>,
    pressed_keys: HashSet<u16>,
    last_poll: Instant,
    event_tx: mpsc::Sender<KeyEvent>,
    buffer: Vec<u8>,
    enabled: bool,
}

impl EvdevListener {
    /// Create a new evdev listener
    pub fn new(event_tx: mpsc::Sender<KeyEvent>) -> Result<Self, EvdevError> {
        let device_paths = find_keyboard_devices()?;
        let mut devices = Vec::new();

        for path in &device_paths {
            match File::open(path) {
                Ok(file) => {
                    // Set non-blocking mode
                    let fd = file.as_raw_fd();
                    unsafe {
                        let flags = libc::fcntl(fd, libc::F_GETFL);
                        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
                    }
                    devices.push(file);
                }
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
                    // Skip devices we can't access
                    continue;
                }
                Err(e) => return Err(EvdevError::Io(e)),
            }
        }

        if devices.is_empty() {
            return Err(EvdevError::PermissionDenied(
                "Cannot access any keyboard devices. Try running with sudo or add user to 'input' group.".to_string(),
            ));
        }

        Ok(Self {
            devices,
            device_paths,
            pressed_keys: HashSet::new(),
            last_poll: Instant::now(),
            event_tx,
            buffer: vec![0u8; INPUT_EVENT_SIZE * 64], // Buffer for multiple events
            enabled: true,
        })
    }

    /// Try to create an evdev listener, return None if not available
    pub fn try_new(event_tx: mpsc::Sender<KeyEvent>) -> Option<Self> {
        Self::new(event_tx).ok()
    }

    /// Check if evdev listener is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Enable or disable the listener
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Get the number of connected devices
    pub fn device_count(&self) -> usize {
        self.devices.len()
    }

    /// Get device paths
    pub fn device_paths(&self) -> &[PathBuf] {
        &self.device_paths
    }

    /// Get currently pressed keys (scancodes)
    pub fn pressed_keys(&self) -> &HashSet<u16> {
        &self.pressed_keys
    }

    /// Poll for keyboard events
    /// Returns the number of events generated
    pub fn poll(&mut self) -> usize {
        if !self.enabled {
            return 0;
        }

        let now = Instant::now();
        let delta_us = now.duration_since(self.last_poll).as_micros() as u64;
        self.last_poll = now;

        let mut event_count = 0;

        for device in &mut self.devices {
            loop {
                match device.read(&mut self.buffer) {
                    Ok(bytes_read) if bytes_read >= INPUT_EVENT_SIZE => {
                        // Process all complete events in the buffer
                        let num_events = bytes_read / INPUT_EVENT_SIZE;
                        for i in 0..num_events {
                            let offset = i * INPUT_EVENT_SIZE;
                            let event_bytes = &self.buffer[offset..offset + INPUT_EVENT_SIZE];

                            // Parse the input event
                            let input_event: InputEvent = unsafe {
                                std::ptr::read(event_bytes.as_ptr() as *const InputEvent)
                            };

                            // We only care about key events
                            if input_event.event_type == EV_KEY {
                                let scancode = input_event.code;
                                let pressed = input_event.value != 0; // 1 = press, 2 = repeat, 0 = release

                                // Skip key repeats (value == 2)
                                if input_event.value == 2 {
                                    continue;
                                }

                                // Track key state
                                if pressed {
                                    if !self.pressed_keys.insert(scancode) {
                                        // Key was already pressed, skip
                                        continue;
                                    }
                                } else {
                                    if !self.pressed_keys.remove(&scancode) {
                                        // Key wasn't pressed, skip
                                        continue;
                                    }
                                }

                                // Create and send the event
                                let event = KeyEvent::new(
                                    KeyCode::new(scancode),
                                    if pressed {
                                        KeyEventType::Press
                                    } else {
                                        KeyEventType::Release
                                    },
                                    now,
                                    delta_us,
                                );
                                let _ = self.event_tx.send(event);
                                event_count += 1;
                            }
                        }
                    }
                    Ok(_) => break, // Not enough bytes for a complete event
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(_) => break, // Other error, stop reading from this device
                }
            }
        }

        event_count
    }

    /// Reset the listener state
    pub fn reset(&mut self) {
        self.pressed_keys.clear();
        self.last_poll = Instant::now();
    }
}

/// Check if evdev is available (Linux only, with device access)
pub fn is_evdev_available() -> bool {
    find_keyboard_devices().is_ok()
}

/// Get a status message about evdev availability
pub fn evdev_status() -> String {
    match find_keyboard_devices() {
        Ok(devices) => format!("{} keyboard device(s) found", devices.len()),
        Err(EvdevError::NoDevices) => "No keyboard devices found".to_string(),
        Err(EvdevError::PermissionDenied(_)) => {
            "Permission denied - run with sudo or add user to 'input' group".to_string()
        }
        Err(e) => format!("Error: {}", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_devices() {
        // This test may fail without proper permissions
        let result = find_keyboard_devices();
        // Just check it doesn't panic
        match result {
            Ok(devices) => println!("Found {} devices", devices.len()),
            Err(e) => println!("Expected error in test environment: {}", e),
        }
    }

    #[test]
    fn test_evdev_status() {
        let status = evdev_status();
        assert!(!status.is_empty());
    }
}
