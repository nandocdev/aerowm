//! Headless outputs: attach, usable area, per-output tiling (Fase 2.8).

mod common;

use common::client::TestClient;
use common::{Act, Answer, Harness, Query};
use std::os::unix::net::UnixStream;

fn pair_client(h: &Harness) -> TestClient {
    let (server_end, client_end) = UnixStream::pair().expect("socketpair");
    h.insert_client(server_end);
    TestClient::connect(client_end)
}

#[test]
fn attach_output_and_usable_area() {
    let h = Harness::boot();
    h.act(Act::AddOutput { name: "test-0".into(), w: 1920, h: 1080 });

    let Answer::Count(n) = h.query(Query::OutputCount) else {
        panic!("expected count")
    };
    assert_eq!(n, 1);

    // No layer-shell surfaces: the whole output is usable for tiling.
    let Answer::Area(area) = h.query(Query::UsableArea(0)) else {
        panic!("expected area")
    };
    assert_eq!(area, (0, 0, 1920, 1080));

    // The output is advertised to clients only once created.
    let client = pair_client(&h);
    assert!(
        client.env.globals.iter().any(|(_, i, _)| i == "wl_output"),
        "wl_output missing; advertised: {:?}",
        client.env.globals
    );
}

#[test]
fn second_output_coexists() {
    let h = Harness::boot();
    h.act(Act::AddOutput { name: "left".into(), w: 1920, h: 1080 });
    h.act(Act::AddOutput { name: "right".into(), w: 1280, h: 720 });

    let Answer::Count(n) = h.query(Query::OutputCount) else {
        panic!("expected count")
    };
    assert_eq!(n, 2);

    let Answer::Area(right) = h.query(Query::UsableArea(1)) else {
        panic!("expected area")
    };
    assert_eq!(right, (0, 0, 1280, 720));
}
