//! Raw evdev-based keyboard listener for Linux
//!
//! This module provides raw scancode detection via evdev, which can detect
//! OEM keys and other special keys that device_query cannot handle.

use super::{KeyCode, KeyEvent, KeyEventType};
use libc;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Read};
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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

// ioctl(EVIOCSCLOCKID) request number, built the same way the kernel's
// _IOW('E', 0xa0, int) macro does so it is correct on every architecture.
#[cfg(any(
    target_arch = "powerpc",
    target_arch = "powerpc64",
    target_arch = "mips",
    target_arch = "mips64",
    target_arch = "sparc",
    target_arch = "sparc64"
))]
const IOC_SIZEBITS: u64 = 13;
#[cfg(any(
    target_arch = "powerpc",
    target_arch = "powerpc64",
    target_arch = "mips",
    target_arch = "mips64",
    target_arch = "sparc",
    target_arch = "sparc64"
))]
const IOC_WRITE: u64 = 4;
#[cfg(not(any(
    target_arch = "powerpc",
    target_arch = "powerpc64",
    target_arch = "mips",
    target_arch = "mips64",
    target_arch = "sparc",
    target_arch = "sparc64"
)))]
const IOC_SIZEBITS: u64 = 14;
#[cfg(not(any(
    target_arch = "powerpc",
    target_arch = "powerpc64",
    target_arch = "mips",
    target_arch = "mips64",
    target_arch = "sparc",
    target_arch = "sparc64"
)))]
const IOC_WRITE: u64 = 1;
const IOC_NRBITS: u64 = 8;
const IOC_TYPEBITS: u64 = 8;

/// `EVIOCSCLOCKID`: ask the kernel to timestamp events with a given clock.
const fn eviocsclockid() -> u64 {
    (IOC_WRITE << (IOC_NRBITS + IOC_TYPEBITS + IOC_SIZEBITS))
        | ((std::mem::size_of::<libc::c_int>() as u64) << (IOC_NRBITS + IOC_TYPEBITS))
        | ((b'E' as u64) << IOC_NRBITS)
        | 0xa0
}

/// Which clock a device's event timestamps are expressed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventClock {
    /// `CLOCK_MONOTONIC` (set via `EVIOCSCLOCKID`) — same clock as `Instant`.
    Monotonic,
    /// `CLOCK_REALTIME` (kernel default) — mapped through `SystemTime`.
    Realtime,
}

/// Maps kernel event timestamps onto the process' `Instant` timeline.
///
/// `Instant` cannot be constructed from a raw clock value, so we capture a
/// (`Instant`, clock) pair once at startup and offset from it.
#[derive(Debug, Clone, Copy)]
struct ClockMapper {
    base_instant: Instant,
    /// Base value of CLOCK_MONOTONIC in microseconds
    base_monotonic_us: i128,
    /// Base value of CLOCK_REALTIME in microseconds
    base_realtime_us: i128,
}

impl ClockMapper {
    fn new() -> Self {
        let base_instant = Instant::now();
        let base_monotonic_us = monotonic_now_us();
        let base_realtime_us = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_micros() as i128)
            .unwrap_or(0);
        Self {
            base_instant,
            base_monotonic_us,
            base_realtime_us,
        }
    }

    /// Convert a kernel `(tv_sec, tv_usec)` timestamp to an `Instant`.
    ///
    /// Timestamps that fall before the mapper was created or noticeably in the
    /// future (clock jumps, suspend/resume) are clamped to `now` so callers
    /// never observe time running backwards by more than the clamp.
    fn to_instant(self, clock: EventClock, tv_sec: i64, tv_usec: i64, now: Instant) -> Instant {
        let event_us = tv_sec as i128 * 1_000_000 + tv_usec as i128;
        let base_us = match clock {
            EventClock::Monotonic => self.base_monotonic_us,
            EventClock::Realtime => self.base_realtime_us,
        };
        let offset_us = event_us - base_us;
        if offset_us < 0 {
            return self.base_instant;
        }
        let candidate = self.base_instant + Duration::from_micros(offset_us as u64);
        if candidate > now + Duration::from_secs(1) {
            now
        } else {
            candidate
        }
    }
}

/// Current CLOCK_MONOTONIC value in microseconds.
fn monotonic_now_us() -> i128 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime only writes to the provided, valid timespec.
    let rc = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    if rc != 0 {
        return 0;
    }
    ts.tv_sec as i128 * 1_000_000 + (ts.tv_nsec / 1000) as i128
}

/// Ask the kernel to timestamp this device's events with CLOCK_MONOTONIC.
fn set_monotonic_clock(file: &File) -> bool {
    let clock: libc::c_int = libc::CLOCK_MONOTONIC;
    // SAFETY: EVIOCSCLOCKID reads a single c_int from the pointer we pass;
    // the fd is a valid, open evdev device.
    let rc = unsafe {
        libc::ioctl(
            file.as_raw_fd(),
            eviocsclockid() as _,
            &clock as *const libc::c_int,
        )
    };
    rc == 0
}

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
fn is_keyboard_device(device_path: &std::path::Path) -> bool {
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
    /// Clock each device's timestamps are expressed in (parallel to `devices`)
    device_clocks: Vec<EventClock>,
    device_paths: Vec<PathBuf>,
    pressed_keys: HashSet<u16>,
    /// Timestamp of the last emitted event (for `delta_us`)
    last_event: Option<Instant>,
    clock: ClockMapper,
    event_tx: mpsc::Sender<KeyEvent>,
    buffer: Vec<u8>,
    enabled: bool,
}

impl EvdevListener {
    /// Create a new evdev listener
    pub fn new(event_tx: mpsc::Sender<KeyEvent>) -> Result<Self, EvdevError> {
        let device_paths = find_keyboard_devices()?;
        let mut devices = Vec::new();
        let mut device_clocks = Vec::new();

        for path in &device_paths {
            match File::open(path) {
                Ok(file) => {
                    // Opened evdev device
                    // Set non-blocking mode
                    let fd = file.as_raw_fd();
                    // SAFETY: fcntl F_GETFL/F_SETFL are safe operations on valid file descriptors.
                    // The fd is valid because it was obtained from a successfully opened File.
                    // O_NONBLOCK flag modification does not affect memory safety.
                    unsafe {
                        let flags = libc::fcntl(fd, libc::F_GETFL);
                        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
                    }
                    // Prefer monotonic timestamps (immune to NTP/clock jumps)
                    let clock = if set_monotonic_clock(&file) {
                        EventClock::Monotonic
                    } else {
                        EventClock::Realtime
                    };
                    devices.push(file);
                    device_clocks.push(clock);
                }
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
                    // Permission denied, skipping device
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
            device_clocks,
            device_paths,
            pressed_keys: HashSet::new(),
            last_event: None,
            clock: ClockMapper::new(),
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

    /// Whether every opened device delivers monotonic (drift-free) timestamps.
    pub fn uses_monotonic_timestamps(&self) -> bool {
        self.device_clocks
            .iter()
            .all(|c| *c == EventClock::Monotonic)
    }

    /// Poll for keyboard events
    /// Returns the number of events generated
    ///
    /// Events are timestamped with the kernel's own event time, not the time
    /// the poll ran, so intervals between events reflect the keyboard rather
    /// than the UI refresh rate.
    pub fn poll(&mut self) -> usize {
        if !self.enabled {
            return 0;
        }

        let now = Instant::now();
        let mut pending: Vec<(Instant, u16, bool)> = Vec::new();

        for (device, &clock) in self.devices.iter_mut().zip(self.device_clocks.iter()) {
            loop {
                match device.read(&mut self.buffer) {
                    Ok(bytes_read) if bytes_read >= INPUT_EVENT_SIZE => {
                        // Process all complete events in the buffer
                        let num_events = bytes_read / INPUT_EVENT_SIZE;
                        for i in 0..num_events {
                            let offset = i * INPUT_EVENT_SIZE;
                            let event_bytes = &self.buffer[offset..offset + INPUT_EVENT_SIZE];

                            // SAFETY: This is safe because:
                            // 1. event_bytes is guaranteed to have exactly INPUT_EVENT_SIZE bytes
                            // 2. InputEvent is #[repr(C)] and matches the kernel's input_event struct layout
                            // 3. The slice was obtained from a buffer read from the kernel evdev interface
                            // 4. All bit patterns are valid for InputEvent's primitive fields
                            let input_event: InputEvent = unsafe {
                                std::ptr::read_unaligned(event_bytes.as_ptr() as *const InputEvent)
                            };

                            // We only care about key events
                            if input_event.event_type != EV_KEY {
                                continue;
                            }
                            // Skip key repeats (value == 2)
                            if input_event.value == 2 {
                                continue;
                            }

                            let scancode = input_event.code;
                            let pressed = input_event.value != 0; // 1 = press, 0 = release

                            // Track key state; drop duplicate presses/releases
                            if pressed {
                                if !self.pressed_keys.insert(scancode) {
                                    continue;
                                }
                            } else if !self.pressed_keys.remove(&scancode) {
                                continue;
                            }

                            let timestamp = self.clock.to_instant(
                                clock,
                                input_event.tv_sec,
                                input_event.tv_usec,
                                now,
                            );
                            pending.push((timestamp, scancode, pressed));
                        }
                    }
                    Ok(_) => break, // Not enough bytes for a complete event
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    // FIXME: Other read errors are silently swallowed. Should distinguish
                    // between transient errors (retry) and fatal ones (disable device).
                    Err(_) => break,
                }
            }
        }

        // Events from several devices may interleave; deliver them in time order.
        pending.sort_by_key(|(ts, _, _)| *ts);

        let mut event_count = 0;
        for (timestamp, scancode, pressed) in pending {
            let delta_us = self
                .last_event
                .map(|last| timestamp.saturating_duration_since(last).as_micros() as u64)
                .unwrap_or(0);
            self.last_event = Some(timestamp);

            let event = KeyEvent::new(
                KeyCode::new(scancode),
                if pressed {
                    KeyEventType::Press
                } else {
                    KeyEventType::Release
                },
                timestamp,
                delta_us,
            );
            if self.event_tx.send(event).is_err() {
                eprintln!("[WARN]  Event channel disconnected, disabling evdev listener");
                self.enabled = false;
                return event_count;
            }
            event_count += 1;
        }

        event_count
    }

    /// Reset the listener state
    pub fn reset(&mut self) {
        self.pressed_keys.clear();
        self.last_event = None;
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

    #[test]
    fn eviocsclockid_matches_kernel_value_on_common_arches() {
        // _IOW('E', 0xa0, int) == 0x400445a0 on x86/arm/riscv
        #[cfg(not(any(
            target_arch = "powerpc",
            target_arch = "powerpc64",
            target_arch = "mips",
            target_arch = "mips64",
            target_arch = "sparc",
            target_arch = "sparc64"
        )))]
        assert_eq!(eviocsclockid(), 0x4004_45a0);
    }

    #[test]
    fn clock_mapper_monotonic_roundtrip() {
        let mapper = ClockMapper::new();
        let now = Instant::now();
        // An event 5ms after the base maps 5ms after base_instant
        let ev_us = mapper.base_monotonic_us + 5_000;
        let ts = mapper.to_instant(
            EventClock::Monotonic,
            (ev_us / 1_000_000) as i64,
            (ev_us % 1_000_000) as i64,
            now + Duration::from_secs(1),
        );
        assert_eq!(
            ts.duration_since(mapper.base_instant),
            Duration::from_millis(5)
        );
    }

    #[test]
    fn clock_mapper_realtime_roundtrip() {
        let mapper = ClockMapper::new();
        let ev_us = mapper.base_realtime_us + 12_345;
        let ts = mapper.to_instant(
            EventClock::Realtime,
            (ev_us / 1_000_000) as i64,
            (ev_us % 1_000_000) as i64,
            Instant::now() + Duration::from_secs(1),
        );
        assert_eq!(
            ts.duration_since(mapper.base_instant),
            Duration::from_micros(12_345)
        );
    }

    #[test]
    fn clock_mapper_clamps_past_and_future() {
        let mapper = ClockMapper::new();
        let now = Instant::now();
        // Before the base: clamped to base_instant
        let past = mapper.to_instant(EventClock::Monotonic, 0, 0, now);
        assert_eq!(past, mapper.base_instant);
        // Far future (clock jump): clamped to now
        let ev_us = mapper.base_monotonic_us + 3_600_000_000;
        let future = mapper.to_instant(
            EventClock::Monotonic,
            (ev_us / 1_000_000) as i64,
            (ev_us % 1_000_000) as i64,
            now,
        );
        assert_eq!(future, now);
    }

    #[test]
    fn input_event_layout_matches_kernel_struct() {
        // struct input_event on 64-bit: timeval (16) + type (2) + code (2) + value (4)
        #[cfg(target_pointer_width = "64")]
        assert_eq!(INPUT_EVENT_SIZE, 24);
    }
}
