//! UIKit windowing and input for GPUI Fast, using its shared Apple text system and Metal renderer.
//!
//! Compile `ios/GPUIHost.swift` into the application's native host and call your Rust
//! entrypoint from `GPUIAppDelegate.start()`. UIKit owns the main run loop.
pub mod unicode;

#[cfg(target_os = "ios")]
mod backend;
#[cfg(target_os = "ios")]
mod bridge;
#[cfg(target_os = "ios")]
pub use backend::{IosPlatform, application, data_directory, run, run_with, shutdown};
