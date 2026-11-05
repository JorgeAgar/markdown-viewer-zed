//! OS integration stays below the shared viewer and renderer.
#[cfg(unix)]
#[path = "unix.rs"]
pub mod ipc;

#[cfg(windows)]
#[path = "windows.rs"]
pub mod ipc;
