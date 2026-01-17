//! Keyboard event handling and state management
//!
//! This module provides the core keyboard input infrastructure for the TestKit.
//!
//! ## Components
//!
//! - [`KeyEvent`] - Represents a single keyboard event with timing data
//! - [`KeyEventType`] - Press or Release event types
//! - [`KeyboardListener`] - Polls for keyboard state changes via device_query
//! - [`KeyboardState`] - Tracks per-key and global state statistics
//! - [`KeyState`] - Per-key metrics (press count, durations, polling intervals)
//! - [`KeyCode`] - Platform-independent key identifier (Linux evdev scancodes)
//! - [`KeyInfo`] - Key metadata including name, label, and position
//! - [`remap`] - Key remapping and OEM/FN key restoration
//!
//! ## Usage
//!
//! ```no_run
//! use keyboard_testkit::keyboard::{KeyboardListener, KeyboardState, KeyEvent};
//! use std::sync::mpsc;
//!
//! // Create event channel
//! let (tx, rx) = mpsc::channel::<KeyEvent>();
//!
//! // Create listener and state tracker
//! let mut listener = KeyboardListener::new(tx);
//! let mut state = KeyboardState::new();
//!
//! // Poll for events
//! listener.poll();
//!
//! // Process received events
//! while let Ok(event) = rx.try_recv() {
//!     state.process_event(&event);
//! }
//! ```

mod event;
mod state;
pub mod keymap;
pub mod remap;

pub use event::{KeyEvent, KeyEventType, KeyboardListener};
pub use state::{KeyState, KeyboardState};
pub use keymap::{KeyCode, KeyInfo, KEYMAP, get_key_info};
