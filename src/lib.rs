//! # Keyboard TestKit
//!
//! A portable, single-executable keyboard testing and diagnostic utility.
//!
//! ## Features
//!
//! - **Polling Rate**: Estimate the report rate (125/250/500 Hz) from evdev timestamps
//! - **Stickiness Detection**: Identify keys that stay down
//! - **Bounce Detection**: Test key hold duration and detect switch chatter (evdev)
//! - **N-Key Rollover (NKRO)**: Count keys registered at once (needs real releases)
//! - **Event Timing**: Gaps between key events (typing rhythm, informational)
//! - **Virtual Keyboard Testing**: Check that injected keys reach the app
//! - **Auto Test**: Guided diagnostic that turns all of the above into findings
//!
//! Results a given input source cannot measure (terminal timestamps, guessed
//! releases) are shown as "n/a" rather than guessed; see
//! [`tests::InputSource`].
//!
//! ## Architecture
//!
//! The crate is organized into the following modules:
//!
//! - [`keyboard`]: Core keyboard input handling, event types, and key mapping
//! - [`tests`]: Test implementations for various keyboard diagnostics
//! - [`ui`]: Terminal UI components using ratatui
//! - [`config`]: Configuration structures for all test parameters
//! - [`report`]: Session report generation and export (JSON, CSV, Markdown, Text)
//!
//! ## Example
//!
//! ```no_run
//! use keyboard_testkit::{Config, keyboard::KeyboardState};
//!
//! // Create default configuration
//! let config = Config::default();
//!
//! // Create keyboard state tracker
//! let mut state = KeyboardState::new();
//!
//! // Process events and track statistics...
//! ```

pub mod config;
pub mod keyboard;
#[cfg(target_os = "linux")]
pub mod mapper;
pub mod report;
pub mod tests;
pub mod ui;
pub mod utils;

pub use config::{config_path, Config, ConfigError, ConfigLoad};
pub use report::SessionReport;
