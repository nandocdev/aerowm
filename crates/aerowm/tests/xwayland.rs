//! XWayland graceful degradation (Fase 2.11).
//!
//! Without the Xwayland binary, spawning the X server must fail as a
//! plain `Err` — the compositor keeps running Wayland-only instead of
//! panicking. Only runs with `--features xwayland`; skipped when the
//! binary exists (spawning a real X server is out of scope for CI).

#![cfg(feature = "xwayland")]

mod common;

use common::{Act, ActResult, Harness};

fn xwayland_present() -> bool {
    std::env::var_os("PATH").map(|paths| std::env::split_paths(&paths).any(|p| p.join("Xwayland").is_file())).unwrap_or(false)
}

#[test]
fn spawn_without_binary_fails_gracefully() {
    if xwayland_present() {
        eprintln!("Xwayland binary present, skipping graceful-failure test");
        return;
    }
    let h = Harness::boot();
    match h.act(Act::TryXwaylandSpawn) {
        ActResult::XwaylandFailed(_) => {}
        ActResult::XwaylandStarted => panic!("spawn succeeded without a binary?"),
        other => panic!("unexpected result: {other:?}"),
    }
}
