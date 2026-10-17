//! Standalone authenticated server; desktop windows are not part of this crate.
pub mod auth;
pub mod control;
pub mod http;
pub mod secrets;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
