//! Keyboard TestKit - Portable keyboard testing and diagnostic utility
//!
//! A comprehensive keyboard testing utility designed for USB portability.
//! Compiles to a single executable with no external dependencies.

pub mod keyboard;
pub mod tests;
pub mod ui;
pub mod config;
pub mod report;

pub use config::Config;
pub use report::SessionReport;
