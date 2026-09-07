//! End-to-end window lifecycle with real xdg-shell clients (Fase 2.7):
//! map → workspace/rules → focus → kill → cleanup, plus layout geometry.

mod common;

use common::client::TestClient;
use common::{wait_for, Act, ActResult, Answer, Harness, Query};
use std::os::unix::net::UnixStream;
use std::time::Duration;

fn pair_client(h: &Harness) -> TestClient {
    let (server_end, client_end) = UnixStream::pair().expect("socketpair");
    h.insert_client(server_end);
    TestClient::connect(client_end)
}

fn count(h: &Harness) -> usize {
    let Answer::Count(n) = h.query(Query::SurfaceCount) else {
        panic!("expected count")
    };
    n
}

fn ws_windows(h: &Harness, i: usize) -> Vec<usize> {
    let Answer::Ids(ids) = h.query(Query::WorkspaceWindows(i)) else {
        panic!("expected ids")
    };
    ids
}

fn focused(h: &Harness) -> Option<usize> {
    let Answer::OptId(id) = h.query(Query::Focused) else {
        panic!("expected opt id")
    };
    id
}

fn load_ok(h: &Harness, config: &str) {
    match h.act(Act::LoadConfig(config.into())) {
        ActResult::Ok => {}
        other => panic!("config failed: {other:?}"),
    }
}

#[test]
fn map_registers_window_and_focus() {
    let h = Harness::boot();
    let mut client = pair_client(&h);
    let _win = client.map_window("test-app", "hello");

    assert!(
        wait_for(Duration::from_secs(5), || count(&h) == 1),
        "server never mapped the toplevel"
    );
    assert_eq!(ws_windows(&h, 0).len(), 1);

    let snap = match h.query(Query::Snapshot) {
        Answer::Snapshot(s) => s,
        _ => panic!("expected snapshot"),
    };
    assert_eq!(snap.workspaces[0].window_count, 1);
    assert_eq!(snap.workspaces[0].focused_window, focused(&h));
    assert!(focused(&h).is_some(), "mapped window takes focus");

    // The server sees the identity the client set.
    let Answer::Apps(apps) = h.query(Query::SurfaceApps) else {
        panic!("expected apps")
    };
    assert_eq!(apps.len(), 1);
    assert_eq!(apps[0].1, "test-app");
}

#[test]
fn rule_routes_window_to_workspace() {
    let h = Harness::boot();
    load_ok(
        &h,
        r#"table.insert(aerowm.rules, { match = { class = "route-me" }, set = { workspace = 2 } })"#,
    );

    let Answer::Count(nrules) = h.query(Query::RuleCount) else {
        panic!("expected rule count")
    };
    assert_eq!(nrules, 1, "config must register exactly one rule");
    let Answer::Count(live) = h.query(Query::EvalRules("route-me".into())) else {
        panic!("expected eval")
    };
    assert_eq!(live, 2, "live engine must match the rule");

    let mut client = pair_client(&h);
    let _win = client.map_window("route-me", "routed");

    assert!(wait_for(Duration::from_secs(5), || count(&h) == 1));
    assert!(ws_windows(&h, 0).is_empty(), "window must leave workspace 1");
    assert_eq!(ws_windows(&h, 1).len(), 1, "rule must route to workspace 2");
    // Routing does not steal the active workspace...
    let Answer::Snapshot(snap) = h.query(Query::Snapshot) else {
        panic!("expected snapshot")
    };
    assert_eq!(snap.active, 0);
    // ...but explicitly switching follows the window.
    h.act(Act::SwitchWorkspace(1));
    let Answer::Snapshot(snap) = h.query(Query::Snapshot) else {
        panic!("expected snapshot")
    };
    assert_eq!(snap.active, 1);
    assert_eq!(snap.workspaces[1].focused_window, focused(&h));
}

#[test]
fn focus_follows_stack_order() {
    let h = Harness::boot();
    let mut client = pair_client(&h);
    let _a = client.map_window("app-a", "a");
    let _b = client.map_window("app-b", "b");
    assert!(wait_for(Duration::from_secs(5), || count(&h) == 2));

    let ids = ws_windows(&h, 0);
    assert_eq!(ids.len(), 2);
    assert_eq!(focused(&h), Some(ids[1]), "newest window focused");

    h.act(Act::FocusPrev);
    assert_eq!(focused(&h), Some(ids[0]));
    h.act(Act::FocusNext);
    assert_eq!(focused(&h), Some(ids[1]));
}

#[test]
fn kill_closes_focused_and_cleanup_reaps() {
    let h = Harness::boot();
    let mut client = pair_client(&h);
    let win = client.map_window("doomed", "bye");
    assert!(wait_for(Duration::from_secs(5), || count(&h) == 1));

    h.act(Act::KillActive);
    assert!(
        wait_for(Duration::from_secs(5), || {
            client.pump();
            client.env.close_seen
        }),
        "client never received xdg_toplevel.close"
    );

    win.destroy(&mut client);
    h.act(Act::CleanupDead);
    assert!(wait_for(Duration::from_secs(5), || count(&h) == 0));
    assert!(ws_windows(&h, 0).is_empty());
    assert_eq!(focused(&h), None);
}

#[test]
fn floating_toggle_pins_window() {
    // Fase 2.10: floating state survives the map path end to end.
    let h = Harness::boot();
    let mut client = pair_client(&h);
    let _win = client.map_window("float-me", "floater");
    assert!(wait_for(Duration::from_secs(5), || count(&h) == 1));
    let id = ws_windows(&h, 0)[0];

    let Answer::Ids(floating) = h.query(Query::FloatingIds) else {
        panic!("expected ids")
    };
    assert!(floating.is_empty(), "tiled by default");

    h.act(Act::ToggleFloating);
    let Answer::Ids(floating) = h.query(Query::FloatingIds) else {
        panic!("expected ids")
    };
    assert_eq!(floating, vec![id]);

    // Floating windows are excluded from the tiling layout: with a single
    // floating window the compositor issues no sized configures for it...
    // (it keeps its user geometry instead).
    h.act(Act::ToggleFloating);
    let Answer::Ids(floating) = h.query(Query::FloatingIds) else {
        panic!("expected ids")
    };
    assert!(floating.is_empty(), "toggle is reversible");
}

#[test]
fn out_of_range_rule_is_ignored_with_warning() {
    let h = Harness::boot();
    load_ok(
        &h,
        r#"table.insert(aerowm.rules, { match = { class = "ghost" }, set = { workspace = 99 } })"#,
    );
    let mut client = pair_client(&h);
    let _win = client.map_window("ghost", "nowhere");
    assert!(wait_for(Duration::from_secs(5), || count(&h) == 1));
    // Stays where it was mapped; no panic, no phantom workspace.
    assert_eq!(ws_windows(&h, 0).len(), 1);
}

#[test]
fn two_windows_tile_full_output() {
    // Geometry chain: Workspace → Layout → space + configure sizes.
    // NOTE: headless windows never attach buffers, so `window.geometry()`
    // stays 0×0 — locations come from the space, sizes from the configure
    // events the server sends (what real clients actually tile by).
    let h = Harness::boot();
    h.act(Act::AddOutput { name: "test-0".into(), w: 1920, h: 1080 });
    let mut client = pair_client(&h);
    let _a = client.map_window("tile-a", "a");
    let _b = client.map_window("tile-b", "b");
    assert!(wait_for(Duration::from_secs(5), || count(&h) == 2));

    // One explicit layout pass → exactly one configure per window, in
    // workspace order. MonadTall default: master left, stack right.
    h.act(Act::ApplyLayout);
    client.settle();
    let tail: Vec<(i32, i32)> = client.env.configures.iter().rev().take(2).rev().copied().collect();
    assert_eq!(tail, vec![(960, 1080), (960, 1080)], "all configures: {:?}", client.env.configures);

    let Answer::Geoms(mut geoms) = h.query(Query::Geometries) else {
        panic!("expected geoms")
    };
    geoms.sort_by_key(|g| g.x);
    assert_eq!(geoms.len(), 2);
    assert_eq!((geoms[0].x, geoms[0].y), (0, 0));
    assert_eq!((geoms[1].x, geoms[1].y), (960, 0));
}
