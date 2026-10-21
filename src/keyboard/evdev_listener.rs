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
use std::path::{Path, PathBuf};
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

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const SYN_REPORT: u16 = 0;
/// The kernel's event buffer overflowed and events were lost.
const SYN_DROPPED: u16 = 3;
const INPUT_EVENT_SIZE: usize = std::mem::size_of::<InputEvent>();

/// Number of key codes the kernel tracks (`KEY_CNT`).
const KEY_CNT: usize = 0x300;
const ULONG_BITS: usize = 8 * std::mem::size_of::<libc::c_ulong>();
/// Size of the `EVIOCGKEY` bitmap in `unsigned long`s.
const KEY_WORDS: usize = KEY_CNT.div_ceil(ULONG_BITS);

/// How often `/dev/input` is rescanned for keyboards that appeared: a
/// replugged cable or a Bluetooth reconnect creates a new event node, and the
/// old file descriptor stays dead. Rescanning reads a few sysfs files, so once
/// a second is cheap and still picks a keyboard up before the user notices.
const RESCAN_INTERVAL: Duration = Duration::from_secs(1);

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
/// `_IOC_READ` is 2 on every architecture Linux supports.
const IOC_READ: u64 = 2;

/// `EVIOCSCLOCKID`: ask the kernel to timestamp events with a given clock.
const fn eviocsclockid() -> u64 {
    (IOC_WRITE << (IOC_NRBITS + IOC_TYPEBITS + IOC_SIZEBITS))
        | ((std::mem::size_of::<libc::c_int>() as u64) << (IOC_NRBITS + IOC_TYPEBITS))
        | ((b'E' as u64) << IOC_NRBITS)
        | 0xa0
}

/// `EVIOCGKEY(len)`: read the device's current key-state bitmap.
const fn eviocgkey(len: usize) -> u64 {
    (IOC_READ << (IOC_NRBITS + IOC_TYPEBITS + IOC_SIZEBITS))
        | ((len as u64) << (IOC_NRBITS + IOC_TYPEBITS))
        | ((b'E' as u64) << IOC_NRBITS)
        | 0x18
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

/// A device's key state as reported by `EVIOCGKEY` (bit n set = key n down).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KeyStateBits([libc::c_ulong; KEY_WORDS]);

impl KeyStateBits {
    fn is_down(&self, code: u16) -> bool {
        let code = code as usize;
        code < KEY_CNT && (self.0[code / ULONG_BITS] >> (code % ULONG_BITS)) & 1 == 1
    }

    /// Every key that is down, lowest code first.
    fn down_keys(&self) -> impl Iterator<Item = u16> + '_ {
        (0..KEY_CNT as u16).filter(|code| self.is_down(*code))
    }
}

/// Ask the kernel which keys are down on this device right now.
fn read_key_state(file: &File) -> Option<KeyStateBits> {
    let mut bits = [0 as libc::c_ulong; KEY_WORDS];
    // SAFETY: EVIOCGKEY(len) writes at most `len` bytes to the pointer we
    // pass, and `len` is exactly the size of `bits`; the fd is an open device.
    let rc = unsafe {
        libc::ioctl(
            file.as_raw_fd(),
            eviocgkey(std::mem::size_of_val(&bits)) as _,
            bits.as_mut_ptr(),
        )
    };
    (rc >= 0).then_some(KeyStateBits(bits))
}

/// What one raw kernel event means for a device's key state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RawOutcome {
    /// Nothing to report (other event types, autorepeat, duplicates)
    Nothing,
    /// A key went down or up on this device
    Key { code: u16, pressed: bool },
    /// Events were lost (`SYN_DROPPED`); the key state must be re-read
    Resync,
}

/// Per-device decoding of the raw event stream.
#[derive(Debug, Default)]
struct DeviceDecoder {
    /// Keys this device reported down
    pressed: HashSet<u16>,
    /// Between `SYN_DROPPED` and the next `SYN_REPORT`: the kernel says this
    /// stretch is incomplete, so it is discarded.
    dropping: bool,
}

impl DeviceDecoder {
    fn feed(&mut self, ev: &InputEvent) -> RawOutcome {
        if ev.event_type == EV_SYN {
            return match ev.code {
                SYN_DROPPED => {
                    self.dropping = true;
                    RawOutcome::Nothing
                }
                SYN_REPORT if self.dropping => {
                    self.dropping = false;
                    RawOutcome::Resync
                }
                _ => RawOutcome::Nothing,
            };
        }
        if self.dropping || ev.event_type != EV_KEY {
            return RawOutcome::Nothing;
        }
        let code = ev.code;
        // Skip key repeats (value == 2) of keys already down. A repeat of a
        // key this device never reported down means it went down before the
        // device was opened (a jammed key, a key held through a replug) and
        // the key state could not be read: it is down now.
        if ev.value == 2 && self.pressed.contains(&code) {
            return RawOutcome::Nothing;
        }
        let pressed = ev.value != 0; // 1 = press, 2 = repeat, 0 = release

        // Drop duplicate presses/releases
        let changed = if pressed {
            self.pressed.insert(code)
        } else {
            self.pressed.remove(&code)
        };
        if changed {
            RawOutcome::Key { code, pressed }
        } else {
            RawOutcome::Nothing
        }
    }

    /// After lost events, bring the key state in line with the kernel's and
    /// return the changes as `(code, pressed)`: releases of keys that are no
    /// longer down, presses of keys that went down meanwhile. Without the
    /// kernel's state (`None`) nothing is known, so every held key is
    /// released: a key shown up while still held is less misleading than one
    /// shown stuck.
    fn resync(&mut self, kernel: Option<&KeyStateBits>) -> Vec<(u16, bool)> {
        let mut released: Vec<u16> = self
            .pressed
            .iter()
            .copied()
            .filter(|code| !kernel.is_some_and(|k| k.is_down(*code)))
            .collect();
        released.sort_unstable();
        for code in &released {
            self.pressed.remove(code);
        }
        let pressed: Vec<u16> = kernel
            .map(|k| {
                k.down_keys()
                    .filter(|code| self.pressed.insert(*code))
                    .collect()
            })
            .unwrap_or_default();
        released
            .into_iter()
            .map(|code| (code, false))
            .chain(pressed.into_iter().map(|code| (code, true)))
            .collect()
    }
}

/// Apply raw events from one device to the shared key state, collecting the
/// key events to emit as `(timestamp, scancode, pressed)`.
///
/// `held` is the set of keys the listener reports down across all devices;
/// a key that is already down on another device is not reported twice.
fn apply_raw_events(
    decoder: &mut DeviceDecoder,
    held: &mut HashSet<u16>,
    raw: &[InputEvent],
    mut key_state: impl FnMut() -> Option<KeyStateBits>,
    mut timestamp: impl FnMut(&InputEvent) -> Instant,
    out: &mut Vec<(Instant, u16, bool)>,
) {
    for ev in raw {
        match decoder.feed(ev) {
            RawOutcome::Nothing => {}
            RawOutcome::Key { code, pressed } => {
                let changed = if pressed {
                    held.insert(code)
                } else {
                    held.remove(&code)
                };
                if changed {
                    out.push((timestamp(ev), code, pressed));
                }
            }
            RawOutcome::Resync => {
                // Changes lost in the overflow happened before this report
                let ts = timestamp(ev);
                for (code, pressed) in decoder.resync(key_state().as_ref()) {
                    let changed = if pressed {
                        held.insert(code)
                    } else {
                        held.remove(&code)
                    };
                    if changed {
                        out.push((ts, code, pressed));
                    }
                }
            }
        }
    }
}

/// Result of draining a device's event queue.
#[derive(Debug, PartialEq, Eq)]
enum DeviceRead {
    /// Queue drained (or nothing pending)
    Drained,
    /// The device is gone (unplugged, Bluetooth link lost, access revoked)
    Gone,
}

/// Read every complete event currently queued on `reader` into `out`.
fn read_raw_events(
    reader: &mut impl Read,
    buffer: &mut [u8],
    out: &mut Vec<InputEvent>,
) -> DeviceRead {
    loop {
        match reader.read(buffer) {
            Ok(bytes_read) if bytes_read >= INPUT_EVENT_SIZE => {
                // Process all complete events in the buffer
                for chunk in buffer[..bytes_read].chunks_exact(INPUT_EVENT_SIZE) {
                    // SAFETY: This is safe because:
                    // 1. chunk is guaranteed to have exactly INPUT_EVENT_SIZE bytes
                    // 2. InputEvent is #[repr(C)] and matches the kernel's input_event struct layout
                    // 3. The slice was obtained from a buffer read from the kernel evdev interface
                    // 4. All bit patterns are valid for InputEvent's primitive fields
                    out.push(unsafe {
                        std::ptr::read_unaligned(chunk.as_ptr() as *const InputEvent)
                    });
                }
            }
            Ok(_) => return DeviceRead::Drained, // Not enough bytes for a complete event
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return DeviceRead::Drained,
            Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
            // ENODEV after an unplug or a revoked fd; any other error also
            // means this descriptor will not deliver events again. The device
            // is dropped and picked up again by the next rescan if it exists.
            Err(_) => return DeviceRead::Gone,
        }
    }
}

/// Human-readable device name from sysfs, e.g. "Logitech USB Keyboard".
fn device_name(path: &Path) -> String {
    let node = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    fs::read_to_string(format!("/sys/class/input/{}/device/name", node))
        .map(|n| n.trim().to_string())
        .ok()
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| path.display().to_string())
}

/// An open keyboard device.
#[derive(Debug)]
struct OpenDevice {
    file: File,
    path: PathBuf,
    name: String,
    /// Clock the device's timestamps are expressed in
    clock: EventClock,
    decoder: DeviceDecoder,
    /// Keys the kernel reported down when the device was opened, not yet
    /// handed to the listener
    down_at_open: Vec<u16>,
    /// A moment before the key state was read: when the keys in
    /// `down_at_open` are known to have been down. Any event read from the
    /// device happened after it, so their releases sort after their presses.
    opened_at: Instant,
    /// Whether the device has delivered key events (see
    /// [`EvdevDeviceChange::Disconnected`])
    typed: bool,
}

/// Open an evdev device for non-blocking reads with monotonic timestamps.
fn open_device(path: &Path) -> io::Result<OpenDevice> {
    let opened_at = Instant::now();
    let file = File::open(path)?;
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
    // A key that is already down sends no press, only autorepeat (dropped
    // as such) or nothing at all: read the kernel's key state so a key
    // jammed before launch, or held through a replug, is seen as down.
    let down_at_open: Vec<u16> = read_key_state(&file)
        .map(|k| k.down_keys().collect())
        .unwrap_or_default();
    let decoder = DeviceDecoder {
        pressed: down_at_open.iter().copied().collect(),
        ..DeviceDecoder::default()
    };
    Ok(OpenDevice {
        file,
        path: path.to_path_buf(),
        name: device_name(path),
        clock,
        decoder,
        down_at_open,
        opened_at,
        typed: false,
    })
}

/// A keyboard appearing or disappearing while the listener runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvdevDeviceChange {
    /// The device stopped delivering events (unplugged, Bluetooth link lost,
    /// access revoked). Keys it held were reported released.
    ///
    /// `typed` says whether it (or an earlier node of the same name, before
    /// a replug) delivered key events this session. Many devices that are
    /// not the keyboard under test have keyboard nodes (a YubiKey, a gaming
    /// or Bluetooth mouse, a barcode scanner); one of those going away says
    /// nothing about the keyboard.
    Disconnected {
        path: PathBuf,
        name: String,
        typed: bool,
    },
    /// A keyboard appeared (replugged, reconnected, newly attached) and is
    /// now read.
    Connected { path: PathBuf, name: String },
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
///
/// Keyboards that disappear (unplug, Bluetooth disconnect) are dropped and
/// their held keys released; `/dev/input` is rescanned every
/// [`RESCAN_INTERVAL`] so a keyboard that comes back (or a new one) is read
/// again. [`EvdevListener::take_device_changes`] reports both.
pub struct EvdevListener {
    devices: Vec<OpenDevice>,
    /// Paths of the open devices (parallel to `devices`)
    device_paths: Vec<PathBuf>,
    /// Keys reported down, across all devices
    pressed_keys: HashSet<u16>,
    /// Timestamp of the last emitted event (for `delta_us`)
    last_event: Option<Instant>,
    clock: ClockMapper,
    event_tx: mpsc::Sender<KeyEvent>,
    buffer: Vec<u8>,
    enabled: bool,
    /// When `/dev/input` was last scanned for new keyboards
    last_scan: Instant,
    /// Devices lost or found since the last `take_device_changes`
    changes: Vec<EvdevDeviceChange>,
    /// Key events not yet sent: presses of keys found down when a device
    /// was opened
    queued: Vec<(Instant, u16, bool)>,
    /// Names of the devices that delivered key events this session
    typed_names: HashSet<String>,
}

impl EvdevListener {
    /// Create a new evdev listener
    pub fn new(event_tx: mpsc::Sender<KeyEvent>) -> Result<Self, EvdevError> {
        let mut devices = Vec::new();
        for path in find_keyboard_devices()? {
            match open_device(&path) {
                Ok(device) => devices.push(device),
                // Permission denied, or the node vanished since the scan:
                // skip it, the rescan retries.
                Err(_) => continue,
            }
        }

        if devices.is_empty() {
            return Err(EvdevError::PermissionDenied(
                "Cannot access any keyboard devices. Try running with sudo or add user to 'input' group.".to_string(),
            ));
        }

        Ok(Self::with_devices(event_tx, devices))
    }

    fn with_devices(event_tx: mpsc::Sender<KeyEvent>, devices: Vec<OpenDevice>) -> Self {
        let mut listener = Self {
            device_paths: Vec::new(),
            devices: Vec::new(),
            pressed_keys: HashSet::new(),
            last_event: None,
            clock: ClockMapper::new(),
            event_tx,
            buffer: vec![0u8; INPUT_EVENT_SIZE * 64], // Buffer for multiple events
            enabled: true,
            last_scan: Instant::now(),
            changes: Vec::new(),
            queued: Vec::new(),
            typed_names: HashSet::new(),
        };
        for device in devices {
            listener.add_device(device);
        }
        listener
    }

    /// Start reading `device`. Keys it already holds are reported pressed
    /// as of when it was opened: not later, when the listener gets to it (at
    /// startup only after every other device was opened), or a release read
    /// meanwhile would sort before its press and leave the key down for good
    /// (the Enter that launched the app, still held as the keyboard opens).
    fn add_device(&mut self, mut device: OpenDevice) {
        for code in std::mem::take(&mut device.down_at_open) {
            device.typed = true;
            if self.pressed_keys.insert(code) {
                self.queued.push((device.opened_at, code, true));
            }
        }
        if device.typed {
            self.typed_names.insert(device.name.clone());
        }
        self.device_paths.push(device.path.clone());
        self.devices.push(device);
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

    /// Get the paths of the connected devices
    pub fn device_paths(&self) -> &[PathBuf] {
        &self.device_paths
    }

    /// Get currently pressed keys (scancodes)
    pub fn pressed_keys(&self) -> &HashSet<u16> {
        &self.pressed_keys
    }

    /// Whether every opened device delivers monotonic (drift-free) timestamps.
    pub fn uses_monotonic_timestamps(&self) -> bool {
        self.devices
            .iter()
            .all(|d| d.clock == EventClock::Monotonic)
    }

    /// Keyboards that disconnected or connected since the last call.
    pub fn take_device_changes(&mut self) -> Vec<EvdevDeviceChange> {
        std::mem::take(&mut self.changes)
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
        let mut pending: Vec<(Instant, u16, bool)> = std::mem::take(&mut self.queued);
        let mut gone = Vec::new();
        let mut raw = Vec::new();
        let mapper = self.clock;

        for (index, device) in self.devices.iter_mut().enumerate() {
            raw.clear();
            let read = read_raw_events(&mut device.file, &mut self.buffer, &mut raw);
            let clock = device.clock;
            let before = pending.len();
            apply_raw_events(
                &mut device.decoder,
                &mut self.pressed_keys,
                &raw,
                || read_key_state(&device.file),
                |ev| mapper.to_instant(clock, ev.tv_sec, ev.tv_usec, now),
                &mut pending,
            );
            if pending.len() > before && !device.typed {
                device.typed = true;
                self.typed_names.insert(device.name.clone());
            }
            if read == DeviceRead::Gone {
                gone.push(index);
            }
        }
        for index in gone.into_iter().rev() {
            self.drop_device(index, now, &mut pending);
        }

        if now.saturating_duration_since(self.last_scan) >= RESCAN_INTERVAL {
            self.last_scan = now;
            let found = find_keyboard_devices().unwrap_or_default();
            self.add_new_devices(&found, open_device);
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

    /// Forget a device that stopped delivering events. Keys it held are
    /// released at `now` so they do not look stuck.
    fn drop_device(&mut self, index: usize, now: Instant, out: &mut Vec<(Instant, u16, bool)>) {
        let device = self.devices.remove(index);
        self.device_paths.remove(index);
        let mut held: Vec<u16> = device.decoder.pressed.iter().copied().collect();
        held.sort_unstable();
        for code in held {
            if self.pressed_keys.remove(&code) {
                out.push((now, code, false));
            }
        }
        let typed = device.typed || self.typed_names.contains(&device.name);
        self.changes.push(EvdevDeviceChange::Disconnected {
            path: device.path,
            name: device.name,
            typed,
        });
    }

    /// Open keyboards in `found` that are not open yet.
    fn add_new_devices(
        &mut self,
        found: &[PathBuf],
        open: impl Fn(&Path) -> io::Result<OpenDevice>,
    ) {
        for path in found {
            if self.device_paths.contains(path) {
                continue;
            }
            // udev may still be setting permissions on a fresh node; a
            // failure here is retried on the next scan.
            if let Ok(device) = open(path) {
                self.changes.push(EvdevDeviceChange::Connected {
                    path: device.path.clone(),
                    name: device.name.clone(),
                });
                self.add_device(device);
            }
        }
    }

    /// Reset the listener state
    pub fn reset(&mut self) {
        self.pressed_keys.clear();
        for device in &mut self.devices {
            device.decoder.pressed.clear();
        }
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

    #[test]
    fn eviocgkey_matches_kernel_value_on_common_arches() {
        // EVIOCGKEY(96) == _IOR('E', 0x18, char[96]) == 0x80604518 on x86/arm/riscv
        #[cfg(not(any(
            target_arch = "powerpc",
            target_arch = "powerpc64",
            target_arch = "mips",
            target_arch = "mips64",
            target_arch = "sparc",
            target_arch = "sparc64"
        )))]
        assert_eq!(eviocgkey(96), 0x8060_4518);
        // The bitmap covers KEY_CNT bits in 96 bytes on every word size
        assert_eq!(KEY_WORDS * std::mem::size_of::<libc::c_ulong>(), 96);
    }

    fn ev(event_type: u16, code: u16, value: i32, usec: i64) -> InputEvent {
        InputEvent {
            tv_sec: 0,
            tv_usec: usec,
            event_type,
            code,
            value,
        }
    }

    fn key(code: u16, value: i32, usec: i64) -> InputEvent {
        ev(EV_KEY, code, value, usec)
    }

    fn syn(code: u16, usec: i64) -> InputEvent {
        ev(EV_SYN, code, 0, usec)
    }

    fn key_state(down: &[u16]) -> KeyStateBits {
        let mut bits = [0 as libc::c_ulong; KEY_WORDS];
        for &code in down {
            bits[code as usize / ULONG_BITS] |= 1 << (code as usize % ULONG_BITS);
        }
        KeyStateBits(bits)
    }

    /// Run raw events through a decoder; timestamps are `base + tv_usec`.
    fn decode(
        decoder: &mut DeviceDecoder,
        held: &mut HashSet<u16>,
        raw: &[InputEvent],
        kernel: Option<KeyStateBits>,
        base: Instant,
    ) -> Vec<(u64, u16, bool)> {
        let mut out = Vec::new();
        apply_raw_events(
            decoder,
            held,
            raw,
            || kernel,
            |e| base + Duration::from_micros(e.tv_usec as u64),
            &mut out,
        );
        out.into_iter()
            .map(|(ts, code, pressed)| (ts.duration_since(base).as_micros() as u64, code, pressed))
            .collect()
    }

    #[test]
    fn decoder_reports_presses_and_releases_and_skips_repeats() {
        let mut d = DeviceDecoder::default();
        let mut held = HashSet::new();
        let base = Instant::now();
        let raw = [
            key(30, 1, 1_000),
            syn(SYN_REPORT, 1_000),
            key(30, 2, 500_000), // autorepeat
            key(30, 1, 500_100), // duplicate press
            key(30, 0, 600_000),
            key(30, 0, 600_100), // duplicate release
        ];
        assert_eq!(
            decode(&mut d, &mut held, &raw, None, base),
            vec![(1_000, 30, true), (600_000, 30, false)]
        );
        assert!(held.is_empty());
    }

    #[test]
    fn syn_dropped_releases_keys_the_kernel_no_longer_holds() {
        // A held, buffer overflow loses A's release: without a resync A
        // would look stuck until it is pressed again.
        let mut d = DeviceDecoder::default();
        let mut held = HashSet::new();
        let base = Instant::now();
        let raw = [
            key(30, 1, 1_000),
            syn(SYN_REPORT, 1_000),
            syn(SYN_DROPPED, 50_000),
            key(31, 1, 50_000), // part of the incomplete packet: discarded
            syn(SYN_REPORT, 50_000),
            key(32, 1, 60_000),
        ];
        let out = decode(&mut d, &mut held, &raw, Some(key_state(&[31])), base);
        // S's press was part of the incomplete packet, but the kernel says
        // S is down after it: the resync reports it, and its release later
        assert_eq!(
            out,
            vec![
                (1_000, 30, true),
                (50_000, 30, false),
                (50_000, 31, true),
                (60_000, 32, true)
            ]
        );
        assert_eq!(held, HashSet::from([31, 32]));
        assert_eq!(
            decode(&mut d, &mut held, &[key(31, 0, 70_000)], None, base),
            vec![(70_000, 31, false)]
        );
    }

    #[test]
    fn syn_dropped_reports_keys_that_went_down_in_the_lost_events() {
        // B pressed (and held) during the overflow: without the kernel's
        // state its press is lost and the key is invisible until released
        let mut d = DeviceDecoder::default();
        let mut held = HashSet::new();
        let base = Instant::now();
        let raw = [
            key(30, 1, 1_000),
            syn(SYN_DROPPED, 50_000),
            syn(SYN_REPORT, 50_000),
        ];
        let out = decode(&mut d, &mut held, &raw, Some(key_state(&[30, 48])), base);
        assert_eq!(out, vec![(1_000, 30, true), (50_000, 48, true)]);
        assert_eq!(held, HashSet::from([30, 48]));
        let out = decode(&mut d, &mut held, &[key(48, 0, 90_000)], None, base);
        assert_eq!(out, vec![(90_000, 48, false)]);
    }

    #[test]
    fn autorepeat_of_a_key_never_seen_down_is_a_press() {
        // Backspace jammed before the device was opened (key state not
        // readable): only its autorepeat arrives. It must show up as down
        // (and later stuck), not stay invisible until the sweep calls it dead.
        let mut d = DeviceDecoder::default();
        let mut held = HashSet::new();
        let base = Instant::now();
        let raw = [key(14, 2, 1_000), key(14, 2, 34_000), key(14, 2, 67_000)];
        assert_eq!(
            decode(&mut d, &mut held, &raw, None, base),
            vec![(1_000, 14, true)]
        );
        assert_eq!(
            decode(&mut d, &mut held, &[key(14, 0, 900_000)], None, base),
            vec![(900_000, 14, false)]
        );
        // Autorepeat of a key that was seen going down stays silent
        let raw = [key(30, 1, 1_000), key(30, 2, 500_000), key(30, 0, 600_000)];
        assert_eq!(
            decode(&mut d, &mut held, &raw, None, base),
            vec![(1_000, 30, true), (600_000, 30, false)]
        );
    }

    #[test]
    fn syn_dropped_keeps_keys_that_are_still_held() {
        // Holding a key through an overflow is normal and must not end the hold
        let mut d = DeviceDecoder::default();
        let mut held = HashSet::new();
        let base = Instant::now();
        let raw = [
            key(42, 1, 1_000),
            syn(SYN_DROPPED, 2_000),
            syn(SYN_REPORT, 2_000),
        ];
        let out = decode(&mut d, &mut held, &raw, Some(key_state(&[42])), base);
        assert_eq!(out, vec![(1_000, 42, true)]);
        assert_eq!(held, HashSet::from([42]));
        // The real release still arrives later
        let out = decode(&mut d, &mut held, &[key(42, 0, 900_000)], None, base);
        assert_eq!(out, vec![(900_000, 42, false)]);
    }

    #[test]
    fn syn_dropped_without_key_state_releases_all_held_keys() {
        let mut d = DeviceDecoder::default();
        let mut held = HashSet::new();
        let base = Instant::now();
        let raw = [
            key(30, 1, 1_000),
            key(31, 1, 2_000),
            syn(SYN_DROPPED, 3_000),
            syn(SYN_REPORT, 3_000),
        ];
        let out = decode(&mut d, &mut held, &raw, None, base);
        assert_eq!(
            out,
            vec![
                (1_000, 30, true),
                (2_000, 31, true),
                (3_000, 30, false),
                (3_000, 31, false)
            ]
        );
        assert!(held.is_empty());
    }

    #[test]
    fn key_held_on_two_devices_is_reported_once() {
        let (mut a, mut b) = (DeviceDecoder::default(), DeviceDecoder::default());
        let mut held = HashSet::new();
        let base = Instant::now();
        assert_eq!(
            decode(&mut a, &mut held, &[key(30, 1, 1_000)], None, base),
            vec![(1_000, 30, true)]
        );
        assert!(decode(&mut b, &mut held, &[key(30, 1, 1_100)], None, base).is_empty());
    }

    fn event_bytes(events: &[InputEvent]) -> Vec<u8> {
        events
            .iter()
            .flat_map(|e| {
                // SAFETY: InputEvent is a plain #[repr(C)] struct of integers
                // without padding on 64-bit targets.
                unsafe {
                    std::slice::from_raw_parts(
                        e as *const InputEvent as *const u8,
                        INPUT_EVENT_SIZE,
                    )
                }
                .to_vec()
            })
            .collect()
    }

    /// A device whose reads return scripted results, then WouldBlock.
    struct ScriptedDevice(std::collections::VecDeque<io::Result<Vec<u8>>>);

    impl Read for ScriptedDevice {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.0.pop_front() {
                Some(Ok(bytes)) => {
                    buf[..bytes.len()].copy_from_slice(&bytes);
                    Ok(bytes.len())
                }
                Some(Err(e)) => Err(e),
                None => Err(io::ErrorKind::WouldBlock.into()),
            }
        }
    }

    #[test]
    fn unplugged_device_is_reported_gone_after_its_last_events() {
        let mut device = ScriptedDevice(
            vec![
                Ok(event_bytes(&[key(30, 1, 1_000), key(30, 0, 2_000)])),
                Err(io::Error::from_raw_os_error(libc::EINTR)),
                Ok(event_bytes(&[key(31, 1, 3_000)])),
                Err(io::Error::from_raw_os_error(libc::ENODEV)),
            ]
            .into(),
        );
        let mut buffer = vec![0u8; INPUT_EVENT_SIZE * 64];
        let mut raw = Vec::new();
        assert_eq!(
            read_raw_events(&mut device, &mut buffer, &mut raw),
            DeviceRead::Gone
        );
        assert_eq!(raw.len(), 3);
        assert_eq!(raw[2].code, 31);
    }

    #[test]
    fn idle_device_is_drained_not_gone() {
        let mut device = ScriptedDevice(Default::default());
        let mut buffer = vec![0u8; INPUT_EVENT_SIZE * 64];
        let mut raw = Vec::new();
        assert_eq!(
            read_raw_events(&mut device, &mut buffer, &mut raw),
            DeviceRead::Drained
        );
        assert!(raw.is_empty());
    }

    fn fake_device(path: &str) -> io::Result<OpenDevice> {
        Ok(OpenDevice {
            file: File::open("/dev/null")?,
            path: PathBuf::from(path),
            name: format!("Test keyboard {}", path),
            clock: EventClock::Monotonic,
            decoder: DeviceDecoder::default(),
            down_at_open: Vec::new(),
            opened_at: Instant::now(),
            typed: false,
        })
    }

    /// A fake device that holds `down` when it is opened.
    fn fake_device_holding(path: &str, down: &[u16]) -> OpenDevice {
        let mut device = fake_device(path).unwrap();
        device.decoder.pressed = down.iter().copied().collect();
        device.down_at_open = down.to_vec();
        device
    }

    #[test]
    fn keys_down_when_a_device_is_opened_are_reported_pressed() {
        // Backspace jammed before launch, and Shift held on a keyboard that
        // is replugged: both are down, and their releases come through
        let (tx, rx) = mpsc::channel();
        let mut listener = EvdevListener::with_devices(
            tx,
            vec![fake_device_holding("/dev/input/event-t0", &[14])],
        );
        listener.add_new_devices(&[PathBuf::from("/dev/input/event-t1")], |p| {
            Ok(fake_device_holding(p.to_str().unwrap(), &[42, 14]))
        });
        assert_eq!(listener.poll(), 2);
        let events: Vec<(u16, KeyEventType)> =
            rx.try_iter().map(|e| (e.key.0, e.event_type)).collect();
        // Backspace held on both devices is one key down
        assert_eq!(
            events,
            vec![(14, KeyEventType::Press), (42, KeyEventType::Press)]
        );
        assert_eq!(listener.pressed_keys(), &HashSet::from([14, 42]));
        // A device found with nothing down reports nothing
        let (tx, rx) = mpsc::channel();
        let mut listener =
            EvdevListener::with_devices(tx, vec![fake_device("/dev/input/event-t2").unwrap()]);
        assert_eq!(listener.poll(), 0);
        assert!(rx.try_iter().next().is_none());
    }

    #[test]
    fn a_key_released_while_the_devices_open_is_not_left_down() {
        // `sudo keyboard-testkit` + Enter: Enter is still down when the
        // keyboard is opened, and comes up while the other devices are being
        // opened, before the listener starts. Its release must follow its
        // press, or the app sees Enter go down for good (a "Stuck key").
        let mut device = fake_device_holding("/dev/input/event-t3", &[28]);
        std::thread::sleep(Duration::from_millis(2));
        let us = monotonic_now_us();
        let at = |event_type, code, value| InputEvent {
            tv_sec: (us / 1_000_000) as i64,
            tv_usec: (us % 1_000_000) as i64,
            event_type,
            code,
            value,
        };
        let mut bytes = Vec::new();
        for ev in [at(EV_KEY, 28, 0), at(EV_SYN, SYN_REPORT, 0)] {
            // SAFETY: InputEvent is #[repr(C)] plain data, read back the
            // same way by read_raw_events
            bytes.extend_from_slice(unsafe {
                std::slice::from_raw_parts(&ev as *const InputEvent as *const u8, INPUT_EVENT_SIZE)
            });
        }
        let file = std::env::temp_dir().join(format!("kt-evdev-{}", std::process::id()));
        fs::write(&file, &bytes).unwrap();
        device.file = File::open(&file).unwrap();
        let _ = fs::remove_file(&file);
        // ...more devices are opened before the listener is created
        std::thread::sleep(Duration::from_millis(2));
        let (tx, rx) = mpsc::channel();
        let mut listener = EvdevListener::with_devices(tx, vec![device]);
        assert_eq!(listener.poll(), 2);
        let events: Vec<(u16, KeyEventType)> =
            rx.try_iter().map(|e| (e.key.0, e.event_type)).collect();
        assert_eq!(
            events,
            vec![(28, KeyEventType::Press), (28, KeyEventType::Release)]
        );
        assert!(listener.pressed_keys().is_empty());
    }

    #[test]
    fn only_devices_that_typed_count_as_a_lost_keyboard() {
        let (tx, _rx) = mpsc::channel();
        let keyboard = fake_device("/dev/input/event-k").unwrap();
        let yubikey = fake_device("/dev/input/event-y").unwrap();
        let mut listener = EvdevListener::with_devices(tx, vec![keyboard, yubikey]);
        listener.devices[0].typed = true;
        listener
            .typed_names
            .insert(listener.devices[0].name.clone());
        let now = Instant::now();
        let mut out = Vec::new();
        // The YubiKey (never typed on) is pulled out, then the keyboard
        listener.drop_device(1, now, &mut out);
        listener.drop_device(0, now, &mut out);
        let typed: Vec<(String, bool)> = listener
            .take_device_changes()
            .into_iter()
            .map(|c| match c {
                EvdevDeviceChange::Disconnected { path, typed, .. } => {
                    (path.display().to_string(), typed)
                }
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            typed,
            vec![
                ("/dev/input/event-y".to_string(), false),
                ("/dev/input/event-k".to_string(), true)
            ]
        );

        // The keyboard comes back as a new node and drops out again before
        // anything is typed on it: still the keyboard
        listener.add_new_devices(&[PathBuf::from("/dev/input/event-k")], |p| {
            fake_device(p.to_str().unwrap())
        });
        listener.take_device_changes();
        listener.drop_device(0, now, &mut out);
        assert!(matches!(
            listener.take_device_changes()[..],
            [EvdevDeviceChange::Disconnected { typed: true, .. }]
        ));
    }

    #[test]
    fn lost_device_releases_its_keys_and_is_reported() {
        let (tx, rx) = mpsc::channel();
        let mut device = fake_device("/dev/input/event-test0").unwrap();
        device.decoder.pressed.insert(30);
        let mut listener = EvdevListener::with_devices(tx, vec![device]);
        listener.pressed_keys.insert(30);

        let now = Instant::now();
        let mut out = Vec::new();
        listener.drop_device(0, now, &mut out);
        assert_eq!(out, vec![(now, 30, false)]);
        assert_eq!(listener.device_count(), 0);
        assert!(listener.device_paths().is_empty());
        assert!(listener.pressed_keys().is_empty());
        assert_eq!(
            listener.take_device_changes(),
            vec![EvdevDeviceChange::Disconnected {
                path: PathBuf::from("/dev/input/event-test0"),
                name: "Test keyboard /dev/input/event-test0".to_string(),
                typed: false,
            }]
        );
        // Changes are handed out once
        assert!(listener.take_device_changes().is_empty());
        drop(rx);
    }

    #[test]
    fn rescan_adds_keyboards_that_appear() {
        let (tx, _rx) = mpsc::channel();
        let first = fake_device("/dev/input/event-test1").unwrap();
        let mut listener = EvdevListener::with_devices(tx, vec![first]);
        let found = vec![
            PathBuf::from("/dev/input/event-test1"),
            PathBuf::from("/dev/input/event-test2"),
        ];
        listener.add_new_devices(&found, |p| fake_device(p.to_str().unwrap()));
        assert_eq!(listener.device_count(), 2);
        assert_eq!(
            listener.take_device_changes(),
            vec![EvdevDeviceChange::Connected {
                path: PathBuf::from("/dev/input/event-test2"),
                name: "Test keyboard /dev/input/event-test2".to_string(),
            }]
        );
        // Already open: nothing new
        listener.add_new_devices(&found, |p| fake_device(p.to_str().unwrap()));
        assert_eq!(listener.device_count(), 2);
        assert!(listener.take_device_changes().is_empty());
        // A node that cannot be opened yet (permissions) is retried later
        listener.add_new_devices(&[PathBuf::from("/dev/input/event-test3")], |_| {
            Err(io::ErrorKind::PermissionDenied.into())
        });
        assert_eq!(listener.device_count(), 2);
        assert!(listener.take_device_changes().is_empty());
    }
}
