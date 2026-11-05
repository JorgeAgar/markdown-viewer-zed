//! OS integration stays below the shared viewer and renderer.
#[cfg(unix)]
#[path = "unix.rs"]
pub mod ipc;
