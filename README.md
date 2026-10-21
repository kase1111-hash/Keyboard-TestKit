# Keyboard TestKit

[![CI](https://github.com/kase1111-hash/Keyboard-TestKit/actions/workflows/ci.yml/badge.svg)](https://github.com/kase1111-hash/Keyboard-TestKit/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

A portable, single-executable keyboard testing and diagnostic utility with a terminal UI. Identify hardware issues, detect software conflicts, and measure keyboard performance—all from a USB drive.

## Features

| Test | Description |
|------|-------------|
| **Polling Rate** | Estimate the keyboard's report rate (125/250/500 Hz) from evdev timestamps; 1000 Hz+ and Bluetooth show no grid |
| **Stickiness** | Detect keys that stay down (stuck) |
| **Bounce** | Identify mechanical key bounce and measure hold/release timing |
| **N-Key Rollover** | Test simultaneous key capability (ghost keys are shown, not auto-detected) |
| **Timing** | Gaps between key events: typing-rhythm statistics, not a keyboard rating |
| **Shortcuts** | Record modifier combos and flag known system shortcuts (needs evdev or the kitty keyboard protocol) |
| **Virtual** | Check that injected keys reach the app, and flag automated (non-human) input |
| **OEM/FN** | OEM key detection, capture, and FN key remapping |
| **Auto** | Guided automatic diagnostic that finds issues for you (dead, stuck and chattering keys, phantom input, limited rollover, polling rate, keyboard disconnects) |

## Screenshots

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ Dashboard │ Polling │ Bounce │ Sticky │ NKRO │ Timing │ Shortcuts │ ...   │
├─────────────────────────────────────────────────────────────────────────────┤
│ ⌨ Keyboard                                                                  │
│ ┌───┬───┬───┬───┬───┬───┬───┬───┬───┬───┬───┬───┬───┬───────┐             │
│ │Esc│ 1 │ 2 │ 3 │ 4 │ 5 │ 6 │ 7 │ 8 │ 9 │ 0 │ - │ = │ Bksp  │             │
│ ├───┴─┬─┴─┬─┴─┬─┴─┬─┴─┬─┴─┬─┴─┬─┴─┬─┴─┬─┴─┬─┴─┬─┴─┬─┴─┬─────┤             │
│ │ Tab │ Q │ W │[E]│ R │ T │ Y │ U │ I │ O │ P │ [ │ ] │  \  │             │
│ ├─────┴┬──┴┬──┴┬──┴┬──┴┬──┴┬──┴┬──┴┬──┴┬──┴┬──┴┬──┴┬──┴─────┤             │
│ │ Caps │[A]│[S]│[D]│ F │ G │ H │ J │ K │ L │ ; │ ' │ Enter  │             │
│ └──────┴───┴───┴───┴───┴───┴───┴───┴───┴───┴───┴───┴────────┘             │
├─────────────────────────────────────────────────────────────────────────────┤
│ Results: Dashboard                                                          │
│ ┌─────────────────────────────────────────────────────────────────────────┐ │
│ │ Session Time    12s                                                     │ │
│ │ Total Events    47                                                      │ │
│ │ Keys Pressed    4                                                       │ │
│ │ Max Rollover    4                                                       │ │
│ │ Est. Poll Rate  125 Hz                                                  │ │
│ └─────────────────────────────────────────────────────────────────────────┘ │
├─────────────────────────────────────────────────────────────────────────────┤
│ RUNNING │ Dashboard │ 00:12 │ Events: 47 │ Tab: switch │ q: quit           │
└─────────────────────────────────────────────────────────────────────────────┘
```

*Real-time keyboard visualization with pressed keys highlighted*

## Installation

### From Source (Recommended)

**Prerequisites:**
- [Rust](https://rustup.rs/) 1.88 or later
- Linux: `libx11-dev`, `libxi-dev`, `libxtst-dev`

```bash
# Clone the repository
git clone https://github.com/kase1111-hash/Keyboard-TestKit.git
cd Keyboard-TestKit

# Build release binary
make release

# Run
./target/release/keyboard-testkit
```

### Linux Dependencies

```bash
# Debian/Ubuntu
sudo apt install libx11-dev libxi-dev libxtst-dev

# Fedora
sudo dnf install libX11-devel libXi-devel libXtst-devel

# Arch
sudo pacman -S libx11 libxi libxtst
```

### System Install (Optional)

```bash
# Install to /usr/local/bin
sudo make install

# Run from anywhere
keyboard-testkit

# Uninstall
sudo make uninstall
```

### Cross-Compile for Windows

```bash
# Install Windows target and MinGW
rustup target add x86_64-pc-windows-gnu
sudo apt install mingw-w64  # Debian/Ubuntu

# Build Windows executable
make windows
# Output: target/x86_64-pc-windows-gnu/release/keyboard-testkit.exe
```

## Usage

```bash
# Run the application
./keyboard-testkit

# Jump straight into the automatic diagnostic
./keyboard-testkit --auto

# Or after installation
keyboard-testkit
```

On Linux, run with `sudo` (or add your user to the `input` group) so the app
can read the keyboard through evdev: that gives raw scancodes, kernel
timestamps and real key-release events. If a keyboard is unplugged or a
wireless keyboard loses its link, the status line says so (a running auto
test stops), the disconnect is listed as a finding, and the keyboard is
picked up again when it comes back. Without evdev the terminal is used as the
input source; terminals that speak the kitty keyboard protocol (kitty,
foot, Ghostty, Alacritty 0.13+, and WezTerm with `enable_kitty_keyboard =
true` in its config; it is off by default) still report key releases, other
terminals only report presses: a key reported again within 200 ms is counted
as a new press (unless it comes faster than a finger can press, which is
autorepeat), a held key is recognised by its autorepeat (so taps resuming
faster than ~8 per second after a short pause can be taken for it: tap counts
are marked approximate), and releases are estimated, so hold times, stuck
keys and rollover cannot be measured: those views show "n/a (terminal
input)" instead of a number.

### Automatic Diagnostic

Press `A` (or start with `--auto`) and follow the on-screen instructions. The
auto test runs five short steps and then writes a ranked list of findings:

| Step | You do | It checks for |
|------|--------|---------------|
| Idle check | Nothing for 3 s | Phantom input, keys already stuck |
| Key sweep | Press every key once (`Ctrl+N` if your keyboard lacks one) | Dead keys, keys that stay down |
| Hold & release | Hold a letter key or Space ~1 s | Release reporting |
| Rollover | Hold 6 or more letter keys at once | Limited N-key rollover |
| Rapid tap | Tap one key quickly with one finger | Switch chatter, polling rate (both evdev only) |

Hold & release and Rollover need real key releases, so they are skipped on
terminals that only report presses. Only evdev sees every key. Through a
terminal the sweep leaves out keys the terminal cannot report (modifiers and
CapsLock without the kitty protocol, the ISO/JIS extra keys) and lists them as
not checked, and a key that never registers is reported as "not registered"
rather than dead. A key that is held down (jammed) through the sweep is
reported as stuck, not dead.

While the auto test runs every key is treated as test input, so `q`, `r`,
`Esc` and the other shortcuts can be verified like any other key. `Ctrl+N`
skips the current step and `Ctrl+C` aborts the run. When the run ends (the
rapid-tap step stops on a timer, a disconnect stops a sweep you may go on
with after a pause) keys stay test input, so a last tap of `r` or `q` does
not wipe the run or quit: stop typing for 2 s and press `Enter` to get the
single-key commands back (the Auto view says so; `Ctrl` chords work
throughout). The findings also appear on the Dashboard (updated live, even
without running the guided test), in the exported report and on stdout when
you quit.

### Keyboard Controls

Keys you press while testing never trigger a command by accident: in the test
views (Polling ... OEM/FN, `2`-`9`) every key is test input except `Tab`,
`Esc` and `Ctrl` chords, and in the Shortcuts view `Ctrl` combos are test
input too (except `Ctrl+C`, which quits). Single-key commands only work on
the Dashboard, Auto and Help screens. Holding a command key runs it once,
also in terminals that report a held key as repeated presses (only moving
between views or Settings values may repeat).

| Key | Where | Action |
|-----|-------|--------|
| `Tab` / `Shift+Tab` | Any view | Next / previous view |
| `Esc` / `Ctrl+C` | Any view | Quit (`Ctrl+C` aborts a running auto test; `Esc` leaves Settings) |
| `Ctrl+P` | Any view but Shortcuts | Pause/resume testing |
| `Ctrl+R` | Any view but Shortcuts | Reset the current test |
| `Ctrl+E` | Any view but Shortcuts | Export report to JSON |
| `v` | Virtual view | Send virtual keys |
| `a` / `f` / `c` | OEM/FN view | Add last unknown key as FN scancode / cycle FN key mode / clear OEM key mappings |
| `A` | Dashboard, Auto, Help | Run the automatic diagnostic |
| `Ctrl+N` | Auto test running | Skip the current auto-test step |
| `Enter` | Auto, after a run | Back to single-key commands (after 2 s without typing) |
| `1-9`, `0` | Dashboard, Auto, Help | Jump to specific view |
| `m` | Dashboard, Auto, Help | Toggle menu shortcuts (free number keys for testing) |
| `Space` | Dashboard, Auto, Help | Pause/resume testing |
| `r` / `R` | Dashboard, Auto, Help | Reset current test / all tests |
| `e` | Dashboard, Auto, Help | Export report to JSON |
| `S` | Dashboard, Auto, Help | Settings (`q`/`Esc` returns) |
| `t` | Dashboard, Auto, Help | Toggle dark/light theme |
| `?` | Dashboard, Auto, Help | Show help |
| `q` | Dashboard, Auto, Help | Quit |

### Test Views

1. **Dashboard** - Overview with session stats and real-time metrics
2. **Polling** - Report-rate estimate from evdev timestamps, plus key-rate statistics
3. **Bounce** - Key bounce detection and hold duration analysis
4. **Sticky** - Stuck key detection with a configurable threshold (needs real key releases)
5. **NKRO** - N-key rollover testing (needs real key releases); the rating counts keys other than modifiers, which are shown next to it; watch Active Keys for ghost keys
6. **Timing** - Informational gaps between key events (the keyboard's report interval is in Polling)
7. **Shortcuts** - Modifier combos pressed, flagging known system shortcuts (n/a where the terminal does not report modifier keys)
8. **Virtual** - Physical vs virtual keyboard comparison
9. **OEM/FN** - OEM key capture and FN key remapping
0. **Auto** - Guided automatic diagnostic with ranked findings
- **Help** - In-app help and key reference (`?`)

## Configuration

The config file is `~/.config/keyboard-testkit/config.toml` on Linux,
`~/Library/Application Support/keyboard-testkit/config.toml` on macOS and
`%APPDATA%\keyboard-testkit\config.toml` on Windows; `--config <path>` uses
another file. Run with `sudo` on Linux, the app still uses your own file
(not root's), and files it creates there stay yours. The mapper service
installed with `--mapper-install` is given that file's path.

Default configuration values (in `src/config.rs`):

| Setting | Default | Description |
|---------|---------|-------------|
| `polling.test_duration_secs` | 10 | Duration for polling rate test |
| `stickiness.stuck_threshold_ms` | 2000 | Time before key is considered stuck |
| `hold_release.bounce_window_ms` | 20 | A same-key press this soon after its release is checked for bounce (20-40, also in Settings; older 5 ms values load as 20) |
| `ui.refresh_rate_hz` | 60 | UI refresh rate |

Out-of-range values in a hand-edited `config.toml` are clamped when it is
loaded (for example `ui.refresh_rate_hz` to 10-240 Hz,
`stickiness.stuck_threshold_ms` to 250-60000 ms and
`polling.test_duration_secs` to 5-3600 s; see `src/config.rs` for all
ranges), and missing entries fall back to their defaults. The Settings view
(`S`) stays within the same ranges. A stuck threshold below 250 ms, such as
the 50 ms that older versions saved as their default, is replaced by the
2000 ms default. Adjusted values are listed on the Dashboard and on exit.

A `config.toml` that cannot be read at all (a syntax error, `theme =
"light"` instead of `"Light"`, `2000.0` where a whole number is expected)
is not used: the status line, the Dashboard and the exit summary say so and
the defaults are in effect. Saving from Settings then keeps the old file as
`config.toml.bak` before writing, so its key mappings are not lost.

A key counts as stuck only while it is still down past the threshold. Keys
you held that long and then released are listed as long holds in the Sticky
view, not as faults (if you did not hold one that long, the key is
sticking). While paused (`Space` or `Ctrl+P`) nothing new is recorded, but
keys that were already down still register their release. Resetting (`r`,
`R`, `Ctrl+R`) forgets keystrokes, not keys that are still down: a jammed
key stays listed as stuck. The reset keystroke itself (Ctrl and R) is not
counted by the tests it clears.

## Diagnostic Logic

The Virtual Keyboard test (`v`, `virtual-send` builds) injects `z`, `x`, `c` and
checks that they reach the app, using your own `v` key press as the physical
baseline:

| Your key press | Injected keys | Diagnosis |
|----------------|---------------|-----------|
| Arrived | Arrived | Input path OK |
| Arrived | Missing | API/Permission Issue (synthetic input blocked) |
| Missing | Any | Inconclusive (e.g. the app was paused) |

With evdev input the check is unavailable, because injected keys bypass the
keyboard device. Faults of individual keys are found by the auto test's sweep.

Bounce and polling-rate results need evdev's kernel timestamps; with terminal
input those views show "n/a (terminal input)" instead of a result. A same-key
re-press inside the bounce window counts as a bounce only when the key was
down for less than 20 ms just before it, or the re-press itself was: no
finger holds a key that briefly, while a switch that chatters closes for a
report or two. A wireless keyboard can deliver a late release together with
the next press, so a fast double letter arrives with no gap at all, but both
of its holds are a finger's and it is not counted. After a key held for half
a second or more, and on a keyboard whose bus has no radio (a laptop's own
keyboard on PS/2, I2C or SPI, or a PS/2 keyboard), any re-press inside the
window counts: the contact dropped out under the finger. On USB, which may
be a 2.4 GHz receiver, a dropout in the middle of a shorter keystroke cannot
be told from a double letter the radio bunched, so it is not counted there.
A key that bounces once is a warning; a key that bounces again is reported
as a fault. On a wireless keyboard (Bluetooth, or a 2.4 GHz receiver
recognized by its name, such as "Receiver" or "Dongle", or, for Logitech
receivers, by the paired keyboard behind it) a radio stall that delivers a
whole keystroke at once looks like a bounce, so its bounces stay a warning
that says so: reset (`R`) and retest on the cable or next to the receiver; if
the key still bounces there, or the keyboard already was on its cable, it is
the switch. "Wireless" alone in a USB keyboard's name does not count, since
tri-mode keyboards keep it on their cable.

Input during the idle check that looks like whole keystrokes (you pressed a
key) is a warning worded as a condition; a key left down or firing again and
again is critical. Stopping the run with `Ctrl+C` during the idle check is
not counted as input. A hold step that ran out of time is only a warning
when no key release arrived during the whole run. Stuck keys
and rollover need real key releases, so a terminal that only reports presses
shows "n/a (terminal input)" there too (and `max_rollover` is `null` in the
exported report).

A keyboard that disconnects (evdev) is reported as a "Connection" warning on
the Dashboard, in the exported report and on exit. An unplug you made on
purpose, or a wireless keyboard going to sleep, looks the same, so the
finding is worded as a condition; resetting all tests (`R`) or starting the
auto test clears it. Only a device that keys were typed on counts: many
other devices have keyboard nodes (a YubiKey, gaming and Bluetooth mice, a
barcode scanner), and one of those going away is only mentioned in the
status line. Keys that are already down when evdev opens a keyboard (a key
jammed before launch, or held while a keyboard reconnects) are read from the
kernel and count as pressed from that moment.

## Export

Press `Ctrl+E` (or `e` on the Dashboard, Auto and Help screens) to export a JSON report with results from all 8 tests (polling, bounce, stickiness, rollover, timing, shortcuts, virtual, and OEM/FN) plus the diagnostic findings:

```json
{
  "metadata": { "generated_at": "1789766772Z", "version": "0.1.0", "duration_secs": 120.4 },
  "summary": {
    "total_events": 1543,
    "max_rollover": 6,
    "estimated_polling_rate_hz": 125.0,
    "issues_detected": 1
  },
  "tests": {
    "polling": [ { "label": "Est. Poll Rate", "value": "125 Hz (report every 8 ms)", "status": "info" } ],
    "rollover": [ ... ],
    "diagnostics": [
      { "label": "Switch bounce", "value": "3 bounce(s) on 1 key(s)", "status": "error" },
      { "label": "", "value": "  E (3 bounces)", "status": "info" }
    ]
  }
}
```

## Build Targets

```bash
make help          # Show all available targets

# Build
make release       # Optimized release binary (default)
make debug         # Debug binary with symbols
make release-full  # Build with virtual-send feature

# Test
make test          # Run tests and clippy
make check         # Check without building

# Distribution
make dist          # Create distribution package
make dist-windows  # Create Windows package
make dist-all      # Build all platforms

# Utilities
make run           # Build and run (debug)
make run-release   # Build and run (release)
make size          # Show binary size analysis
make fmt           # Format code
make doc           # Generate documentation
make clean         # Remove build artifacts
```

## Binary Size

The release binary is optimized for portability:

- **Size:** ~700-800 KB (stripped, LTO enabled)
- **Dependencies:** None (statically linked)
- **Portability:** Single executable, runs from USB drive

## Platform Support

| Platform | Status | Notes |
|----------|--------|-------|
| Linux | Full support | Primary development platform |
| Windows | Cross-compile | Requires MinGW for building |
| macOS | Cross-compile | Requires toolchain setup |

## Contributing

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

### Development

```bash
# Format code before committing
make fmt

# Run lints
cargo clippy --all-targets -- -D warnings

# Run tests
cargo test
```

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE.md) file for details.

## Acknowledgments

- [Ratatui](https://ratatui.rs/) - Terminal UI framework
- [device_query](https://crates.io/crates/device_query) - Cross-platform keyboard input
- [Crossterm](https://crates.io/crates/crossterm) - Terminal manipulation

---

**Keyboard TestKit** - Diagnose your keyboard, anywhere.
