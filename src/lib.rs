//! # Keyboard TestKit
//!
//! A portable, single-executable keyboard testing and diagnostic utility.
//!
//! ## Features
//!
//! - **Polling Rate Testing**: Measure keyboard polling frequency (Hz) with jitter detection
//! - **Stickiness Detection**: Identify stuck or unresponsive keys
//! - **Bounce Detection**: Test key hold duration and detect mechanical bounce
//! - **N-Key Rollover (NKRO)**: Measure simultaneous key capability and ghosting
//! - **Latency Measurement**: Track input-to-system latency per-key and globally
//! - **Virtual Keyboard Testing**: Compare physical vs virtual key events
//!
//! ## Architecture
//!
//! The crate is organized into the following modules:
//!
//! - [`keyboard`]: Core keyboard input handling, event types, and key mapping
//! - [`tests`]: Test implementations for various keyboard diagnostics
//! - [`ui`]: Terminal UI components using ratatui
//! - [`config`]: Configuration structures for all test parameters
//! - [`report`]: Session report generation and JSON export
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

pub mod keyboard;
pub mod tests;
pub mod ui;
pub mod config;
pub mod report;
pub mod utils;

pub use config::{Config, ConfigError, config_path};
pub use report::SessionReport;
