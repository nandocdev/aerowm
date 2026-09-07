//! AeroWM compositor library.
//!
//! The binary (`src/main.rs`) is a thin entry point; everything testable
//! lives here so integration tests (`tests/`) can boot a headless
//! compositor with in-process Wayland clients.

pub mod backend;
pub mod handlers;
pub mod input;
pub mod ipc;
pub mod session;
pub mod state;
pub mod wayland_socket;
