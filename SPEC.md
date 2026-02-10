# Keyboard TestKit - Specification Sheet

## Overview

Keyboard TestKit is a comprehensive keyboard testing and diagnostic utility designed to evaluate keyboard hardware performance, detect software conflicts, and provide real-time feedback on keyboard behavior.

---

## Core Features

### 1. Key Polling Rate Test

**Purpose:** Measure how frequently the keyboard reports key states to the computer.

| Specification | Details |
|---------------|---------|
| Measurement Unit | Hertz (Hz) |
| Display Format | Real-time Hz reading + average over test duration |
| Target Range | 125Hz - 8000Hz |
| Accuracy | ±1ms timing precision |
| Test Duration | User-configurable (default: 10 seconds) |

**Implementation Requirements:**
- Timestamp each keypress event at OS level
- Calculate intervals between consecutive reports
- Display instantaneous and rolling average polling rates
- Flag inconsistent polling (jitter detection)
- Compare against manufacturer-stated specs

---

### 2. Key Stickiness Detection

**Purpose:** Identify keys that remain registered after physical release (stuck keys).

| Specification | Details |
|---------------|---------|
| Detection Method | Monitor key-up event timing |
| Threshold | Configurable (default: 50ms after physical release) |
| Alert Type | Visual highlight |
| Logging | Record all sticky key incidents with timestamps |

**Implementation Requirements:**
- Track press duration for each key
- Compare against expected release timing
- Detect keys that fail to send key-up events
- Identify intermittent sticking patterns
- Generate report of problematic keys

---

### 3. Hold Down & Release Test

**Purpose:** Verify proper key registration during sustained holds and clean release behavior.

| Specification | Details |
|---------------|---------|
| Repeat Rate Test | Measure characters-per-second during hold |
| Repeat Delay Test | Time from initial press to first repeat |
| Release Accuracy | Verify single key-up event on release |
| Bounce Detection | Identify multiple triggers on single press/release |

**Implementation Requirements:**
- Display real-time hold duration counter
- Measure and display repeat rate (typically 2-30 chars/sec)
- Measure initial delay before repeat starts (typically 250-1000ms)
- Detect key bounce (multiple rapid on/off within 5ms)
- Visual representation of key state over time (waveform view)

---

### 4. Virtual Keyboard Signal Testing

**Purpose:** Differentiate between hardware and software keyboard issues by sending synthetic key events.

| Specification | Details |
|---------------|---------|
| Input Methods | Windows SendInput API / Linux uinput / evdev |
| Event Types | Key down, key up, key press (combined) |
| Target Selection | Specific application or system-wide |
| Comparison Mode | Side-by-side physical vs virtual results |

**Implementation Requirements:**
- Send virtual keypress events programmatically
- Monitor if virtual keys are received by target application
- Compare physical keyboard behavior to virtual keyboard
- Identify when physical keys fail but virtual keys work (hardware issue)
- Identify when both fail (software/driver issue)

**Diagnostic Logic:**
```
Physical Works + Virtual Works = Keyboard OK
Physical Fails + Virtual Works = Hardware Issue
Physical Fails + Virtual Fails = Software/Driver Issue
Physical Works + Virtual Fails = API/Permission Issue
```

---

### 5. Shortcut Conflict Detection

**Purpose:** Identify when keystrokes are intercepted by system or application shortcuts.

| Specification | Details |
|---------------|---------|
| Detection Scope | System-wide global hotkeys |
| Monitoring Method | Low-level keyboard hooks |
| Real-time Alert | Visual indicator when shortcut intercepts key |
| Conflict Database | Store known conflicts for reference |

**Implementation Requirements:**
- Install low-level keyboard hook to intercept all key events
- Detect when key events don't reach target application
- Identify which process registered the conflicting hotkey
- Real-time "interrupt indicator" showing interception occurred
- Quick-test mode: rapidly test all key combinations

---

### 6. Program Shortcut Listing

**Purpose:** Display all currently registered global hotkeys on the system.

| Specification | Details |
|---------------|---------|
| Data Sources | Windows: RegisterHotKey registry, WM_HOTKEY messages |
| | Linux: gsettings, dconf, window manager configs |
| Display Format | Table with Key Combo, Owning Process, Description |
| Export Options | CSV, JSON, plain text |

**Implementation Requirements:**
- Enumerate all registered global hotkeys
- Identify owning process for each hotkey
- Group by application/category
- Search/filter functionality
- Detect conflicts (same combo registered multiple times)

**Sample Output:**
```
| Shortcut       | Application      | Action              |
|----------------|------------------|---------------------|
| Ctrl+Shift+S   | ShareX           | Screenshot region   |
| Ctrl+Alt+T     | Terminal         | Open terminal       |
| Win+Shift+S    | Windows          | Snipping tool       |
| F12            | Discord          | Toggle overlay      |
```

---

### 7. Shortcut Usage Warning Light

**Purpose:** Provide immediate visual feedback when a shortcut key combination is triggered.

| Specification | Details |
|---------------|---------|
| Indicator Type | In-app visual indicator |
| Display Duration | 2-3 seconds (configurable) |
| Information Shown | Key combo detected |

> **Current status:** The shortcut detection tracks when system hotkeys are pressed. The floating overlay / system tray indicator described below is planned but not yet implemented. Currently, shortcut conflicts are displayed within the terminal UI's Shortcuts view.

**Planned Implementation (not yet available):**
- Overlay window that appears on shortcut detection
- Display the captured key combination
- Show which application consumed the shortcut
- Fade-out animation after display duration
- Option to click for more details
- History log of recent shortcut activations

---

## Additional Standard Testing Features

### 8. N-Key Rollover (NKRO) Test

**Purpose:** Determine maximum simultaneous key registration capability.

| Specification | Details |
|---------------|---------|
| Test Method | Count maximum concurrent key-down states |
| Display | Visual keyboard with active key highlighting |
| Reporting | X-Key Rollover rating (2KRO, 6KRO, NKRO) |
| Matrix Analysis | Identify which key combinations fail |

---

### 9. Ghosting Detection

**Purpose:** Identify phantom key registrations when pressing multiple keys.

| Specification | Details |
|---------------|---------|
| Detection Method | Compare pressed keys vs registered keys |
| Alert Type | Highlight phantom keys in different color |
| Common Combos | Pre-test gaming combinations (WASD+Space+Shift) |

---

### 10. Inter-Event Timing Measurement

**Purpose:** Measure the time interval between consecutive keyboard polling events.

| Specification | Details |
|---------------|---------|
| Measurement | Milliseconds (ms) |
| Method | Poll-to-poll interval timing |
| Per-Key Testing | Individual timing per key option |

> **Note:** This test measures inter-event polling intervals (time between consecutive keyboard reports), not true end-to-end input latency (physical switch actuation to software registration). True end-to-end latency measurement requires specialized hardware.

---

## User Interface Requirements

### Main Dashboard
- Visual keyboard layout (currently hardcoded ANSI US layout)
- Real-time key highlighting on press
- Status indicators for each test module
- Tab-based navigation between test views

### Test Results Panel
- Per-key statistics and health indicators
- Export functionality (JSON, CSV, Markdown, Text)

### Configuration
- Settings managed via TOML configuration file
- Platform-specific config file locations (see README)
- Threshold configurations editable in config file

> **Note:** An in-app settings panel for runtime configuration is planned but not yet implemented. Currently, all settings are managed through the TOML configuration file.

---

## Technical Requirements

### Platform Support
| Platform | Minimum Version |
|----------|-----------------|
| Windows  | Windows 10 1903+ |
| Linux    | Kernel 5.0+ with evdev |
| macOS    | macOS 11+ (Big Sur) |

### Dependencies
- Low-level keyboard hook access (admin/root may be required)
- USB device enumeration
- High-resolution timer access (sub-millisecond)
- Window overlay capabilities

### Performance Targets
| Metric | Target |
|--------|--------|
| CPU Usage | <2% during passive monitoring |
| Memory | <50MB base, <100MB during intensive tests |
| Latency Overhead | <0.5ms added latency |
| Startup Time | <2 seconds |

---

## Data Logging & Reports

### Log Format
```json
{
  "timestamp": "2025-12-25T10:30:00Z",
  "event_type": "key_press",
  "key_code": 65,
  "key_name": "A",
  "scan_code": 30,
  "duration_ms": 120,
  "polling_rate_hz": 1000,
  "flags": ["shift_held", "shortcut_detected"]
}
```

### Report Generation
- Summary statistics per testing session
- Problem key identification with severity ratings
- Comparison against baseline/previous tests
- Recommendations for keyboard issues

---

## Security Considerations

- Keyboard hooks must not log passwords or sensitive input
- Configurable exclusion list for secure applications
- No network transmission of keystroke data
- Secure local storage with optional encryption
- Clear indication when logging is active

---

## Research Sources

- [PassMark KeyboardTest](https://www.passmark.com/products/keytest/) - Industry standard testing tool
- [Keyboard Test Utility](https://www.filehorse.com/download-keyboard-test-utility/) - Rollover and ghosting testing
- [VIA Keyboard Configuration](https://caniusevia.com/) - QMK keyboard configuration
- [LTT Labs Keyboard Testing](https://www.lttlabs.com/blog/2025/12/03/how-labs-conducts-keyboard-testing) - Professional testing methodology
- [KeyboardTester.com](https://www.keyboardtester.com/) - Browser-based testing reference
- [Tom's Hardware - Hotkey Conflicts](https://www.tomshardware.com/software/windows/how-to-resolve-hotkey-conflicts-in-windows) - Shortcut conflict resolution
- [PowerToys Shortcut Conflict Request](https://github.com/microsoft/PowerToys/issues/28197) - Feature reference
