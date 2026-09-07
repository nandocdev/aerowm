//! Headless boot + protocol registry (Fase 2, punto 7).
//!
//! Boots a real `Display<AerowmState>` with no backend and checks, through
//! an in-process client, that the compositor advertises the globals its
//! delegates implement.

mod common;

use common::client::TestClient;
use common::Harness;
use std::os::unix::net::UnixStream;

fn pair_client(h: &Harness) -> TestClient {
    let (server_end, client_end) = UnixStream::pair().expect("socketpair");
    h.insert_client(server_end);
    TestClient::connect(client_end)
}

#[test]
fn headless_boot_advertises_core_globals() {
    let h = Harness::boot();
    let client = pair_client(&h);
    let names: Vec<String> = client.env.globals.iter().map(|(_, i, _)| i.clone()).collect();
    for required in ["wl_compositor", "wl_shm", "xdg_wm_base", "wl_seat"] {
        assert!(
            names.iter().any(|n| n == required),
            "missing global {required}; advertised: {names:?}"
        );
    }
}

#[test]
fn headless_boot_advertises_protocol_globals() {
    // xdg-decoration, fractional-scale and viewporter delegates are wired
    // (ROADMAP Fase Wayland); a regression here means a `*State` was
    // dropped or a delegate removed.
    let h = Harness::boot();
    let client = pair_client(&h);
    let names: Vec<String> = client.env.globals.iter().map(|(_, i, _)| i.clone()).collect();
    for required in [
        "zxdg_decoration_manager_v1",
        "wp_fractional_scale_manager_v1",
        "wp_viewporter",
    ] {
        assert!(
            names.iter().any(|n| n == required),
            "missing global {required}; advertised: {names:?}"
        );
    }
}

#[test]
fn headless_boot_has_default_workspaces() {
    let h = Harness::boot();
    let common::Answer::Snapshot(snap) = h.query(common::Query::Snapshot) else {
        panic!("expected snapshot");
    };
    assert_eq!(snap.workspaces.len(), 4);
    assert_eq!(snap.active, 0);
}
