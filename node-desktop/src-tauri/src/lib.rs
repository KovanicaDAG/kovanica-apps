//! Library half of the Linux node supervisor.
//!
//! The Tauri window in `main.rs` is a thin shell over these modules so the
//! policy, monitor, and process tests do not link GTK.

pub mod monitor;
pub mod policy;
pub mod redact;
pub mod supervisor;
