//! Forkstr's configuration, execution, capture, and terminal components.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("Forkstr currently supports macOS and Linux only");

pub mod capture;
pub mod config;
pub mod model;
pub mod process;
pub mod report;
pub mod runner;
pub mod skill;
pub mod terminal;
