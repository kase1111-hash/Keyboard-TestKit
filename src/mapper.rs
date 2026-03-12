//! System-level key mapper daemon for special keyboard keys
//!
//! This module provides a background key remapping service that:
//! - Grabs input devices via evdev for exclusive access
//! - Creates a virtual keyboard via uinput to emit remapped keys
//! - Runs as a systemd service for startup persistence
//!
//! ## How It Works
//!
//! 1. Discovers keyboard input devices in `/dev/input/`
//! 2. Grabs exclusive access to the target device (prevents duplicate events)
//! 3. Creates a virtual keyboard device via `/dev/uinput`
//! 4. Reads raw input events, applies configured remappings, and emits via uinput
//!
//! ## ASUS G14 Support
//!
//! Pre-configured mappings for ASUS ROG Zephyrus G14 special keys:
//! - ROG key → configurable action (default: launch terminal or Super key)
//! - Fan profile key → mapped to configurable target
//! - AURA key → keyboard backlight cycle
//! - Microphone mute → proper mic mute scancode
//!
//! ## Usage
//!
//! Run as daemon:
//! ```bash
//! sudo keyboard-testkit --mapper
//! ```
//!
//! Run with ASUS G14 preset:
//! ```bash
//! sudo keyboard-testkit --mapper --preset asus-g14
//! ```

use crate::config::Config;
use crate::keyboard::keymap::KeyCode;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use libc;

/// Error type for mapper operations
#[derive(Debug)]
pub enum MapperError {
    /// No suitable input devices found
    NoDevices,
    /// Permission denied (needs root/input group)
    PermissionDenied(String),
    /// Failed to create uinput device
    UinputFailed(String),
    /// IO error
    Io(io::Error),
    /// Device not found
    DeviceNotFound(String),
}

impl std::fmt::Display for MapperError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MapperError::NoDevices => write!(f, "No suitable keyboard input devices found"),
            MapperError::PermissionDenied(msg) => write!(f, "Permission denied: {}", msg),
            MapperError::UinputFailed(msg) => write!(f, "Failed to create uinput device: {}", msg),
            MapperError::Io(e) => write!(f, "IO error: {}", e),
            MapperError::DeviceNotFound(msg) => write!(f, "Device not found: {}", msg),
        }
    }
}

impl std::error::Error for MapperError {}

impl From<io::Error> for MapperError {
    fn from(e: io::Error) -> Self {
        if e.kind() == io::ErrorKind::PermissionDenied {
            MapperError::PermissionDenied(e.to_string())
        } else {
            MapperError::Io(e)
        }
    }
}

/// A raw input event from the kernel (matches struct input_event)
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
const EV_MSC: u16 = 0x04;
const INPUT_EVENT_SIZE: usize = std::mem::size_of::<InputEvent>();

// uinput ioctl constants
const UINPUT_MAX_NAME_SIZE: usize = 80;
const UI_SET_EVBIT: libc::c_ulong = 0x40045564;
const UI_SET_KEYBIT: libc::c_ulong = 0x40045565;
const UI_DEV_CREATE: libc::c_ulong = 0x5501;
const UI_DEV_DESTROY: libc::c_ulong = 0x5502;

// EVIOCGRAB ioctl for exclusive device access
const EVIOCGRAB: libc::c_ulong = 0x40044590;

/// uinput_user_dev structure for device setup
#[repr(C)]
struct UinputUserDev {
    name: [u8; UINPUT_MAX_NAME_SIZE],
    id_bustype: u16,
    id_vendor: u16,
    id_product: u16,
    id_version: u16,
    ff_effects_max: u32,
    absmax: [i32; 64],
    absmin: [i32; 64],
    absfuzz: [i32; 64],
    absflat: [i32; 64],
}

/// Vendor-specific key mapping preset
#[derive(Debug, Clone)]
pub struct MapperPreset {
    /// Human-readable name
    pub name: String,
    /// Description of what this preset does
    pub description: String,
    /// Key mappings: source scancode → target scancode
    pub mappings: HashMap<u16, u16>,
    /// Device name pattern to match (substring match against /sys device name)
    pub device_match: Option<String>,
}

impl MapperPreset {
    /// Create the ASUS ROG Zephyrus G14 preset
    pub fn asus_g14() -> Self {
        let mut mappings = HashMap::new();

        // ROG key (PROG1 = 148) → Super/Meta key for app launcher
        mappings.insert(148, 125); // KEY_PROG1 → KEY_LSUPER

        // AURA key (PROG2 = 149) → Keyboard backlight toggle
        mappings.insert(149, 228); // KEY_PROG2 → KEY_KBDILLUMTOGGLE

        // Fan profile key (PROG3 = 202) → mapped to F13 (unused F-key, can be
        // bound to custom scripts via desktop environment)
        mappings.insert(202, 183); // KEY_PROG3 → KEY_F13

        // Mic mute key — ensure it emits standard KEY_MICMUTE (248)
        // Some ASUS keyboards send a vendor-specific code instead
        mappings.insert(248, 248); // Pass through (identity, ensures recognition)

        // Screenshot key (if present) → Print Screen
        mappings.insert(414, 99); // KEY_SELECTIVE_SCREENSHOT → KEY_PRINT

        Self {
            name: "ASUS ROG Zephyrus G14".to_string(),
            description: "Maps ROG key, AURA, fan profile, mic mute, and screenshot keys"
                .to_string(),
            mappings,
            device_match: Some("asus".to_string()),
        }
    }

    /// Create a generic laptop preset
    pub fn generic_laptop() -> Self {
        let mut mappings = HashMap::new();

        // Common laptop keys that may need remapping
        // Mic mute passthrough
        mappings.insert(248, 248);
        // Airplane mode passthrough
        mappings.insert(247, 247);

        Self {
            name: "Generic Laptop".to_string(),
            description: "Basic mappings for common laptop special keys".to_string(),
            mappings,
            device_match: None,
        }
    }

    /// Get a preset by name
    pub fn by_name(name: &str) -> Option<Self> {
        match name.to_lowercase().as_str() {
            "asus-g14" | "asus_g14" | "g14" | "asus" => Some(Self::asus_g14()),
            "generic" | "laptop" => Some(Self::generic_laptop()),
            _ => None,
        }
    }

    /// List all available preset names
    pub fn available() -> Vec<(&'static str, &'static str)> {
        vec![
            ("asus-g14", "ASUS ROG Zephyrus G14/G15 special keys"),
            ("generic", "Generic laptop special keys"),
        ]
    }
}

/// The key mapper daemon
pub struct KeyMapper {
    /// Key remappings: source scancode → target scancode
    mappings: HashMap<u16, u16>,
    /// Input device file
    input_device: File,
    /// Input device path (for logging)
    input_path: PathBuf,
    /// uinput device file descriptor
    uinput_fd: i32,
    /// Read buffer
    buffer: Vec<u8>,
    /// Whether the mapper is running
    running: Arc<AtomicBool>,
}

impl KeyMapper {
    /// Create a new key mapper for the specified device with given mappings
    pub fn new(
        device_path: PathBuf,
        mappings: HashMap<u16, u16>,
        running: Arc<AtomicBool>,
    ) -> Result<Self, MapperError> {
        // Open the input device
        let input_device = File::open(&device_path).map_err(|e| {
            if e.kind() == io::ErrorKind::PermissionDenied {
                MapperError::PermissionDenied(format!(
                    "Cannot open {}. Run with sudo or add user to 'input' group.",
                    device_path.display()
                ))
            } else {
                MapperError::Io(e)
            }
        })?;

        // Create the uinput virtual device
        let uinput_fd = Self::create_uinput_device()?;

        // Grab exclusive access to the input device
        let input_fd = input_device.as_raw_fd();
        // SAFETY: EVIOCGRAB is a safe ioctl that grants exclusive access to an evdev device.
        // The fd is valid because we just opened it successfully.
        let grab_result = unsafe { libc::ioctl(input_fd, EVIOCGRAB, 1 as libc::c_int) };
        if grab_result < 0 {
            // Clean up uinput on failure
            unsafe {
                libc::ioctl(uinput_fd, UI_DEV_DESTROY);
                libc::close(uinput_fd);
            }
            return Err(MapperError::PermissionDenied(format!(
                "Failed to grab exclusive access to {}. Is another mapper running?",
                device_path.display()
            )));
        }

        eprintln!(
            "Key mapper active on {} with {} mapping(s)",
            device_path.display(),
            mappings.len()
        );

        Ok(Self {
            mappings,
            input_device,
            input_path: device_path,
            uinput_fd,
            buffer: vec![0u8; INPUT_EVENT_SIZE * 64],
            running,
        })
    }

    /// Create a uinput virtual keyboard device
    fn create_uinput_device() -> Result<i32, MapperError> {
        // Open uinput
        let uinput_path = if std::path::Path::new("/dev/uinput").exists() {
            "/dev/uinput"
        } else {
            "/dev/input/uinput"
        };

        let uinput_cstr = std::ffi::CString::new(uinput_path)
            .map_err(|_| MapperError::UinputFailed("Invalid uinput path".to_string()))?;

        let fd = unsafe { libc::open(uinput_cstr.as_ptr(), libc::O_WRONLY | libc::O_NONBLOCK) };

        if fd < 0 {
            return Err(MapperError::UinputFailed(format!(
                "Cannot open {}. Ensure the uinput module is loaded: sudo modprobe uinput",
                uinput_path
            )));
        }

        // SAFETY: All ioctl calls below use valid fd and kernel-defined constants.
        // UI_SET_EVBIT/UI_SET_KEYBIT configure which event types and keys the
        // virtual device supports before creation.
        unsafe {
            // Enable EV_KEY and EV_SYN event types
            if libc::ioctl(fd, UI_SET_EVBIT, EV_KEY as libc::c_int) < 0 {
                libc::close(fd);
                return Err(MapperError::UinputFailed(
                    "Failed to set EV_KEY".to_string(),
                ));
            }
            if libc::ioctl(fd, UI_SET_EVBIT, EV_SYN as libc::c_int) < 0 {
                libc::close(fd);
                return Err(MapperError::UinputFailed(
                    "Failed to set EV_SYN".to_string(),
                ));
            }
            // Also support EV_MSC for scancode passthrough
            if libc::ioctl(fd, UI_SET_EVBIT, EV_MSC as libc::c_int) < 0 {
                libc::close(fd);
                return Err(MapperError::UinputFailed(
                    "Failed to set EV_MSC".to_string(),
                ));
            }

            // Enable all key codes 0-767 (KEY_MAX)
            for key in 0..768 {
                libc::ioctl(fd, UI_SET_KEYBIT, key as libc::c_int);
            }
        }

        // Set up the device info
        let mut dev = UinputUserDev {
            name: [0u8; UINPUT_MAX_NAME_SIZE],
            id_bustype: 0x03, // BUS_USB
            id_vendor: 0x1234,
            id_product: 0x5678,
            id_version: 1,
            ff_effects_max: 0,
            absmax: [0; 64],
            absmin: [0; 64],
            absfuzz: [0; 64],
            absflat: [0; 64],
        };

        let dev_name = b"Keyboard-TestKit Virtual Keyboard";
        let name_len = dev_name.len().min(UINPUT_MAX_NAME_SIZE - 1);
        dev.name[..name_len].copy_from_slice(&dev_name[..name_len]);

        // Write device info
        // SAFETY: Writing the uinput_user_dev struct to the fd is the standard
        // way to configure a uinput device before creation.
        let dev_bytes = unsafe {
            std::slice::from_raw_parts(
                &dev as *const UinputUserDev as *const u8,
                std::mem::size_of::<UinputUserDev>(),
            )
        };

        let written = unsafe {
            libc::write(fd, dev_bytes.as_ptr() as *const libc::c_void, dev_bytes.len())
        };

        if written < 0 {
            unsafe { libc::close(fd) };
            return Err(MapperError::UinputFailed(
                "Failed to write device info".to_string(),
            ));
        }

        // Create the device
        // SAFETY: UI_DEV_CREATE finalizes the virtual device. The fd is valid
        // and the device info has been written.
        if unsafe { libc::ioctl(fd, UI_DEV_CREATE) } < 0 {
            unsafe { libc::close(fd) };
            return Err(MapperError::UinputFailed(
                "Failed to create uinput device".to_string(),
            ));
        }

        eprintln!("Created virtual keyboard device");
        Ok(fd)
    }

    /// Write an input event to the uinput device
    fn emit_event(&self, event_type: u16, code: u16, value: i32) {
        let event = InputEvent {
            tv_sec: 0,
            tv_usec: 0,
            event_type,
            code,
            value,
        };

        // SAFETY: Writing a valid InputEvent struct to the uinput fd emits
        // the corresponding input event. The fd is valid and the struct is repr(C).
        unsafe {
            libc::write(
                self.uinput_fd,
                &event as *const InputEvent as *const libc::c_void,
                INPUT_EVENT_SIZE,
            );
        }
    }

    /// Emit a SYN_REPORT to synchronize events
    #[allow(dead_code)]
    fn emit_syn(&self) {
        self.emit_event(EV_SYN, 0, 0);
    }

    /// Run the mapper loop — blocks until stopped
    pub fn run(&mut self) -> Result<(), MapperError> {
        eprintln!("Key mapper daemon running on {}", self.input_path.display());

        while self.running.load(Ordering::SeqCst) {
            match self.input_device.read(&mut self.buffer) {
                Ok(bytes_read) if bytes_read >= INPUT_EVENT_SIZE => {
                    let num_events = bytes_read / INPUT_EVENT_SIZE;
                    for i in 0..num_events {
                        let offset = i * INPUT_EVENT_SIZE;
                        let event_bytes = &self.buffer[offset..offset + INPUT_EVENT_SIZE];

                        // SAFETY: event_bytes has exactly INPUT_EVENT_SIZE bytes,
                        // InputEvent is #[repr(C)] matching the kernel struct,
                        // and all bit patterns are valid for its primitive fields.
                        let event: InputEvent =
                            unsafe { std::ptr::read(event_bytes.as_ptr() as *const InputEvent) };

                        if event.event_type == EV_KEY {
                            // Check if this key should be remapped
                            let output_code = self
                                .mappings
                                .get(&event.code)
                                .copied()
                                .unwrap_or(event.code);

                            if output_code != event.code {
                                // Key remapped
                            }

                            self.emit_event(EV_KEY, output_code, event.value);
                        } else {
                            // Forward non-key events unchanged (SYN, MSC, etc.)
                            self.emit_event(event.event_type, event.code, event.value);
                        }
                    }
                }
                Ok(_) => {
                    // Incomplete read, wait a bit
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {
                    continue;
                }
                Err(e) => {
                    eprintln!("Read error on {}: {}", self.input_path.display(), e);
                    return Err(MapperError::Io(e));
                }
            }
        }

        eprintln!("Key mapper daemon stopped");
        Ok(())
    }
}

impl Drop for KeyMapper {
    fn drop(&mut self) {
        // Release the grabbed device
        let input_fd = self.input_device.as_raw_fd();
        // SAFETY: Releasing the grab and destroying the uinput device are
        // cleanup operations on valid file descriptors.
        unsafe {
            libc::ioctl(input_fd, EVIOCGRAB, 0 as libc::c_int);
            libc::ioctl(self.uinput_fd, UI_DEV_DESTROY);
            libc::close(self.uinput_fd);
        }
        eprintln!("Key mapper cleaned up for {}", self.input_path.display());
    }
}

/// Find keyboard devices, optionally filtering by name pattern
pub fn find_mapper_devices(name_pattern: Option<&str>) -> Result<Vec<(PathBuf, String)>, MapperError> {
    let input_dir = PathBuf::from("/dev/input");
    if !input_dir.exists() {
        return Err(MapperError::NoDevices);
    }

    let mut devices = Vec::new();

    let entries = fs::read_dir(&input_dir).map_err(MapperError::Io)?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        if !name.starts_with("event") {
            continue;
        }

        // Get device name from sysfs
        let sysfs_name_path = format!("/sys/class/input/{}/device/name", name);
        let dev_name = fs::read_to_string(&sysfs_name_path)
            .unwrap_or_default()
            .trim()
            .to_string();

        if dev_name.is_empty() {
            continue;
        }

        // Check if this is a keyboard by capabilities
        let caps_path = format!("/sys/class/input/{}/device/capabilities/key", name);
        if let Ok(caps) = fs::read_to_string(&caps_path) {
            let trimmed = caps.trim();
            if !trimmed.is_empty() && trimmed != "0" {
                let total_bits: u32 = trimmed
                    .split_whitespace()
                    .filter_map(|hex| u64::from_str_radix(hex, 16).ok())
                    .map(|n| n.count_ones())
                    .sum();

                // Keyboards typically have 50+ key capabilities
                if total_bits > 20 {
                    // Apply name filter if provided
                    if let Some(pattern) = name_pattern {
                        if dev_name.to_lowercase().contains(&pattern.to_lowercase()) {
                            devices.push((path, dev_name));
                        }
                    } else {
                        devices.push((path, dev_name));
                    }
                }
            }
        }
    }

    if devices.is_empty() {
        return Err(MapperError::NoDevices);
    }

    Ok(devices)
}

/// Run the key mapper daemon with the given configuration
pub fn run_mapper(
    preset_name: Option<&str>,
    device_path: Option<PathBuf>,
    extra_mappings: &[(u16, u16)],
    running: Arc<AtomicBool>,
) -> Result<(), MapperError> {
    // Load preset mappings
    let mut mappings = HashMap::new();

    if let Some(name) = preset_name {
        if let Some(preset) = MapperPreset::by_name(name) {
            eprintln!("Loaded preset: {} - {}", preset.name, preset.description);
            mappings.extend(preset.mappings);
        } else {
            eprintln!("Unknown preset '{}'. Available presets:", name);
            for (pname, desc) in MapperPreset::available() {
                eprintln!("  {} - {}", pname, desc);
            }
        }
    }

    // Also load from config file if available
    if let Ok(config) = Config::load() {
        for (from, to) in &config.oem_keys.key_mappings {
            mappings.insert(*from, *to);
        }
    }

    // Apply extra mappings (override preset/config)
    for (from, to) in extra_mappings {
        mappings.insert(*from, *to);
    }

    if mappings.is_empty() {
        eprintln!("No key mappings configured. Use --preset or configure mappings in config.toml");
        eprintln!("Available presets:");
        for (name, desc) in MapperPreset::available() {
            eprintln!("  --preset {} : {}", name, desc);
        }
        return Ok(());
    }

    // Find the target device
    let target_path = if let Some(path) = device_path {
        path
    } else {
        // Auto-detect: try to find ASUS device first if using ASUS preset
        let pattern = preset_name.and_then(|n| {
            MapperPreset::by_name(n)
                .and_then(|p| p.device_match)
        });

        let devices = find_mapper_devices(pattern.as_deref())?;

        // If pattern didn't match, try without filter
        let devices = if devices.is_empty() {
            find_mapper_devices(None)?
        } else {
            devices
        };

        eprintln!("Found {} input device(s):", devices.len());
        for (path, name) in &devices {
            eprintln!("  {} - {}", path.display(), name);
        }

        // Use the first matching device
        devices
            .into_iter()
            .next()
            .map(|(path, _)| path)
            .ok_or(MapperError::NoDevices)?
    };

    eprintln!("Using device: {}", target_path.display());
    eprintln!("Active mappings:");
    for (from, to) in &mappings {
        let from_info = crate::keyboard::keymap::get_key_info(KeyCode::new(*from));
        let to_info = crate::keyboard::keymap::get_key_info(KeyCode::new(*to));
        eprintln!(
            "  {} (0x{:03X}) → {} (0x{:03X})",
            from_info.name,
            from,
            to_info.name,
            to
        );
    }

    let mut mapper = KeyMapper::new(target_path, mappings, running)?;
    mapper.run()
}

/// Generate a systemd service unit file content
pub fn generate_systemd_service(preset: Option<&str>) -> String {
    let preset_arg = preset
        .map(|p| format!(" --preset {}", p))
        .unwrap_or_default();

    format!(
        r#"[Unit]
Description=Keyboard TestKit Key Mapper Daemon
Documentation=https://github.com/kase1111-hash/Keyboard-TestKit
After=systemd-udevd.service
Wants=systemd-udevd.service

[Service]
Type=simple
ExecStart=/usr/local/bin/keyboard-testkit --mapper{preset_arg}
Restart=on-failure
RestartSec=3
StandardOutput=journal
StandardError=journal

# Security hardening
ProtectSystem=strict
ProtectHome=read-only
PrivateTmp=true
NoNewPrivileges=false
# Needs access to /dev/input and /dev/uinput
DeviceAllow=/dev/input/* rw
DeviceAllow=/dev/uinput rw
SupplementaryGroups=input

[Install]
WantedBy=multi-user.target
"#
    )
}

/// Generate a udev rule for the uinput module and device permissions
pub fn generate_udev_rules() -> String {
    r#"# Allow the input group to access uinput for key remapping
KERNEL=="uinput", GROUP="input", MODE="0660"
"#
    .to_string()
}

/// Install the mapper as a systemd service
pub fn install_service(preset: Option<&str>) -> Result<(), MapperError> {
    let service_content = generate_systemd_service(preset);
    let service_path = "/etc/systemd/system/keyboard-testkit-mapper.service";

    // Write service file
    fs::write(service_path, &service_content).map_err(|e| {
        if e.kind() == io::ErrorKind::PermissionDenied {
            MapperError::PermissionDenied(
                "Cannot write to /etc/systemd/system/. Run with sudo.".to_string(),
            )
        } else {
            MapperError::Io(e)
        }
    })?;

    // Write udev rule
    let udev_path = "/etc/udev/rules.d/99-keyboard-testkit.rules";
    let udev_content = generate_udev_rules();
    fs::write(udev_path, &udev_content).map_err(|e| {
        if e.kind() == io::ErrorKind::PermissionDenied {
            MapperError::PermissionDenied(
                "Cannot write to /etc/udev/rules.d/. Run with sudo.".to_string(),
            )
        } else {
            MapperError::Io(e)
        }
    })?;

    // Copy the binary to /usr/local/bin
    let current_exe = std::env::current_exe().map_err(MapperError::Io)?;
    let target_bin = "/usr/local/bin/keyboard-testkit";
    fs::copy(&current_exe, target_bin).map_err(|e| {
        if e.kind() == io::ErrorKind::PermissionDenied {
            MapperError::PermissionDenied(
                "Cannot copy to /usr/local/bin/. Run with sudo.".to_string(),
            )
        } else {
            MapperError::Io(e)
        }
    })?;

    // Ensure the binary is executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        fs::set_permissions(target_bin, perms).map_err(MapperError::Io)?;
    }

    eprintln!("Service installed to {}", service_path);
    eprintln!("Udev rule installed to {}", udev_path);
    eprintln!("Binary installed to {}", target_bin);
    eprintln!("");
    eprintln!("To enable and start the service:");
    eprintln!("  sudo systemctl daemon-reload");
    eprintln!("  sudo systemctl enable keyboard-testkit-mapper");
    eprintln!("  sudo systemctl start keyboard-testkit-mapper");
    eprintln!("");
    eprintln!("To check status:");
    eprintln!("  sudo systemctl status keyboard-testkit-mapper");
    eprintln!("  journalctl -u keyboard-testkit-mapper -f");

    Ok(())
}

/// Uninstall the mapper systemd service
pub fn uninstall_service() -> Result<(), MapperError> {
    let service_path = "/etc/systemd/system/keyboard-testkit-mapper.service";
    let udev_path = "/etc/udev/rules.d/99-keyboard-testkit.rules";

    // Stop and disable the service (ignore errors if not running)
    let _ = std::process::Command::new("systemctl")
        .args(["stop", "keyboard-testkit-mapper"])
        .status();
    let _ = std::process::Command::new("systemctl")
        .args(["disable", "keyboard-testkit-mapper"])
        .status();

    // Remove files
    if std::path::Path::new(service_path).exists() {
        fs::remove_file(service_path).map_err(MapperError::Io)?;
        eprintln!("Removed {}", service_path);
    }
    if std::path::Path::new(udev_path).exists() {
        fs::remove_file(udev_path).map_err(MapperError::Io)?;
        eprintln!("Removed {}", udev_path);
    }

    // Reload systemd
    let _ = std::process::Command::new("systemctl")
        .arg("daemon-reload")
        .status();

    eprintln!("Service uninstalled successfully");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_asus_g14_preset() {
        let preset = MapperPreset::asus_g14();
        assert_eq!(preset.name, "ASUS ROG Zephyrus G14");
        assert!(!preset.mappings.is_empty());

        // ROG key should map to Super
        assert_eq!(preset.mappings.get(&148), Some(&125));
        // AURA should map to backlight toggle
        assert_eq!(preset.mappings.get(&149), Some(&228));
        // Fan profile should map to F13
        assert_eq!(preset.mappings.get(&202), Some(&183));
    }

    #[test]
    fn test_preset_by_name() {
        assert!(MapperPreset::by_name("asus-g14").is_some());
        assert!(MapperPreset::by_name("asus").is_some());
        assert!(MapperPreset::by_name("g14").is_some());
        assert!(MapperPreset::by_name("generic").is_some());
        assert!(MapperPreset::by_name("nonexistent").is_none());
    }

    #[test]
    fn test_available_presets() {
        let presets = MapperPreset::available();
        assert!(presets.len() >= 2);
        assert!(presets.iter().any(|(name, _)| *name == "asus-g14"));
        assert!(presets.iter().any(|(name, _)| *name == "generic"));
    }

    #[test]
    fn test_generate_systemd_service() {
        let service = generate_systemd_service(Some("asus-g14"));
        assert!(service.contains("[Unit]"));
        assert!(service.contains("[Service]"));
        assert!(service.contains("[Install]"));
        assert!(service.contains("--mapper"));
        assert!(service.contains("--preset asus-g14"));
    }

    #[test]
    fn test_generate_systemd_service_no_preset() {
        let service = generate_systemd_service(None);
        assert!(service.contains("--mapper"));
        assert!(!service.contains("--preset"));
    }

    #[test]
    fn test_generate_udev_rules() {
        let rules = generate_udev_rules();
        assert!(rules.contains("uinput"));
        assert!(rules.contains("input"));
    }

    #[test]
    fn test_generic_preset() {
        let preset = MapperPreset::generic_laptop();
        assert_eq!(preset.name, "Generic Laptop");
        assert!(preset.device_match.is_none());
    }
}
