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
| Display Format | Estimated report rate, plus key-rate statistics |
| Target Range | 125, 250 and 500 Hz (a 8, 4 or 2 ms report grid); 1000 Hz and above, and Bluetooth, show no grid and no rate is claimed |
| Accuracy | The grid step itself; needs evdev kernel timestamps (terminal input shows "n/a") |
| Test Duration | User-configurable (default: 10 seconds), and at least 24 gaps between key events |

**Implementation Requirements:**
- Timestamp each key event with the kernel's time (evdev)
- Infer the report interval from the quantization of the gaps between key
  events (presses and releases); gaps that are all multiples of a coarser
  period (a Bluetooth connection interval) are not a report grid
- Key rates, gap spread and the Timing view are typing rhythm, shown for
  information, never as a keyboard rating

> **Current status:** jitter detection and comparison against manufacturer
> specs are not implemented.

---

### 2. Key Stickiness Detection

**Purpose:** Identify keys that remain registered after physical release (stuck keys).

| Specification | Details |
|---------------|---------|
| Detection Method | Monitor key-up event timing |
| Threshold | Configurable (default: 2000ms held without release) |
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
| Hold Times | Per-key hold durations (needs real key releases) |
| Release Accuracy | Verify single key-up event on release |
| Bounce Detection | Same key re-pressed less than 20-40 ms (configurable) after its release, or less than 40 ms after its press, with the hold before it or the re-press's own hold under 20 ms (judged when the re-press ends), the hold before it 500 ms or more, or the keyboard on a bus with no radio (i8042, I2C, SPI, platform); evdev kernel timestamps only |
| Press Rate | Presses per second of any keys (typing pace) |

**Implementation Requirements:**
- Display real-time hold duration counter
- Detect key bounce from kernel timestamps; keys whose releases come from
  the driver (laptop hotkeys with a force-release quirk) are not judged
- A fast re-press between two finger-length holds is not a bounce: a
  wireless link can deliver a late release together with the next press.
  After a hold of 500 ms or more, or on a keyboard whose bus carries no
  radio (a laptop's own keyboard, a PS/2 one), it is the contact dropping
  out and is a bounce
- A single bounce is a warning; a key that bounces again is a fault, except
  on a wireless keyboard (Bluetooth bus type from `EVIOCGID`, a receiver
  name, or a HID device paired behind a receiver), where a radio stall that
  delivers a whole keystroke at once looks the same: it stays a warning that
  says so

> **Current status:** autorepeat rate and delay are not measured (evdev and
> the terminal listener drop autorepeat events), and there is no waveform
> view.

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

> **Current status:** the Shortcuts view records the modifier combos that
> reach the app and flags known system shortcuts; interception itself is not
> detected. Modifier keys only arrive through evdev or the kitty keyboard
> protocol, so with other terminal input the view shows "n/a".

---

### 6. Program Shortcut Listing

> **Current status:** This feature is planned but not yet implemented. The current Shortcuts test detects when system hotkeys are pressed during testing, but does not enumerate registered global hotkeys or identify owning processes.

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
| Test Method | Count maximum concurrent key-down states (needs real key releases) |
| Display | Visual keyboard with active key highlighting |
| Reporting | X-Key Rollover rating (2KRO, 6KRO, NKRO) counting keys other than modifiers, which are shown next to it |
| Matrix Analysis | Identify which key combinations fail (not implemented) |

---

### 9. Ghosting Detection

**Purpose:** Identify phantom key registrations when pressing multiple keys.

| Specification | Details |
|---------------|---------|
| Detection Method | Compare pressed keys vs registered keys |
| Alert Type | Highlight phantom keys in different color |
| Common Combos | Pre-test gaming combinations (WASD+Space+Shift) |

> **Current status:** ghosting is not detected automatically: a ghost key
> arrives as an ordinary press, and nothing tells the app which keys are
> really held. The NKRO view lists the active keys for the user to judge.

---

### 10. Inter-Event Timing Measurement

**Purpose:** Show the gaps between consecutive key events.

| Specification | Details |
|---------------|---------|
| Measurement | Milliseconds (ms) |
| Method | Time between consecutive key events |
| Per-Key Testing | Individual timing per key option |

> **Note:** These gaps are set by the typing rhythm, so they are shown for
> information and never rate the keyboard. The keyboard's report interval is
> estimated by the polling test. True end-to-end latency (switch actuation to
> software registration) needs specialized hardware.

---

## User Interface Requirements

### Main Dashboard
- Visual keyboard layout (ANSI, ISO or JIS, detected from the system)
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
- In-app Settings view (`S`) for the thresholds, refresh rate, theme and FN
  key mode, saved to the same file

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
- High-resolution timer access (sub-millisecond)

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
