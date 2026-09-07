//! Shared headless harness for AeroWM integration tests (Fase 2).
//!
//! A real `Display<AerowmState>` runs on a background thread with a
//! continuous pump (`dispatch_clients` + `flush_clients`), so tests can use
//! blocking Wayland client calls (`roundtrip`) without deadlocks.
//! In-process clients are attached with the production `ClientState`
//! (the server panics on unknown client data, see
//! `handlers::compositor::client_compositor_state`).

//! Support surface is intentionally broader than any single test binary
//! (each `tests/*.rs` is compiled separately with this module inlined).
#![allow(dead_code)]

use aerowm::handlers::compositor::ClientState;
use aerowm::state::AerowmState;
use aerowm_ipc::CompositorSnapshot;
use aerowm_lua::ScriptEngine;

use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use wayland_server::Display;

pub mod client;

// --- Queries & actions (plain data across threads) ---------------------------

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)] // shared support: each test binary uses a subset
pub struct Geom {
    pub id: usize,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug)]
#[allow(dead_code)] // shared support: each test binary uses a subset
pub enum Query {
    Snapshot,
    SurfaceCount,
    /// Window ids (as usize) on a workspace index.
    WorkspaceWindows(usize),
    /// `(id, app_id)` pairs as the server sees them (empty app_id = unset).
    SurfaceApps,
    /// Number of entries in `aerowm.rules` (did the config load?).
    RuleCount,
    /// Live `evaluate_rules(class)` on the compositor engine → workspace (1-based) or 0.
    EvalRules(String),
    /// Ids of all floating windows.
    FloatingIds,
    /// Focused window id on the active workspace.
    Focused,
    /// `(id, x, y, w, h)` per mapped window in the space.
    Geometries,
    OutputCount,
    /// Usable tiling area of output `idx` (x, y, w, h).
    UsableArea(usize),
}

#[derive(Debug)]
#[allow(dead_code)] // shared support: each test binary uses a subset
pub enum Answer {
    Snapshot(CompositorSnapshot),
    Count(usize),
    Ids(Vec<usize>),
    /// `(id, app_id)` pairs.
    Apps(Vec<(usize, String)>),
    OptId(Option<usize>),
    Geoms(Vec<Geom>),
    Area((i32, i32, u32, u32)),
}

#[derive(Debug)]
#[allow(dead_code)] // shared support: each test binary uses a subset
pub enum Act {
    LoadConfig(String),
    ToggleFloating,
    FocusNext,
    FocusPrev,
    KillActive,
    SwitchWorkspace(usize),
    CleanupDead,
    ApplyLayout,
    /// Attach a headless output `(name, w, h)` and make it the active one.
    AddOutput { name: String, w: i32, h: i32 },
    /// Try to spawn XWayland; only meaningful when the binary is missing
    /// (must fail gracefully, never panic).
    TryXwaylandSpawn,
}

#[derive(Debug)]
#[allow(dead_code)] // shared support: each test binary uses a subset
pub enum ActResult {
    Ok,
    ConfigErr(String),
    XwaylandFailed(String),
    XwaylandStarted,
}

enum Request {
    InsertClient(UnixStream),
    Query(Query, mpsc::Sender<Answer>),
    Act(Act, mpsc::Sender<ActResult>),
    Stop,
}

// --- Harness -----------------------------------------------------------------

pub struct Harness {
    tx: mpsc::Sender<Request>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Harness {
    pub fn boot() -> Self {
        let (tx, rx) = mpsc::channel::<Request>();
        let thread = thread::spawn(move || {
            let mut display = Display::<AerowmState>::new().expect("headless display");
            let mut dh = display.handle();
            let engine = ScriptEngine::new().expect("script engine");
            let mut state = AerowmState::new(&dh, engine);
            let mut outputs: Vec<smithay::output::Output> = Vec::new();
            loop {
                let _ = display.dispatch_clients(&mut state);
                let _ = display.flush_clients();
                match rx.try_recv() {
                    Ok(Request::Stop) | Err(mpsc::TryRecvError::Disconnected) => break,
                    Ok(Request::InsertClient(stream)) => {
                        let data = std::sync::Arc::new(ClientState::default());
                        if dh.insert_client(stream, data).is_err() {
                            break;
                        }
                    }
                    Ok(Request::Query(q, reply)) => {
                        let _ = reply.send(answer(&state, &outputs, q));
                    }
                    Ok(Request::Act(a, reply)) => {
                        let _ = reply.send(apply(&dh, &mut display, &mut state, &mut outputs, a));
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                }
                thread::sleep(Duration::from_millis(1));
            }
        });
        Self {
            tx,
            thread: Some(thread),
        }
    }

    pub fn insert_client(&self, stream: UnixStream) {
        self.tx.send(Request::InsertClient(stream)).unwrap();
    }

    pub fn query(&self, q: Query) -> Answer {
        let (tx, rx) = mpsc::channel();
        self.tx.send(Request::Query(q, tx)).unwrap();
        rx.recv_timeout(Duration::from_secs(10))
            .expect("server thread hung or died")
    }

    pub fn act(&self, a: Act) -> ActResult {
        let (tx, rx) = mpsc::channel();
        self.tx.send(Request::Act(a, tx)).unwrap();
        rx.recv_timeout(Duration::from_secs(10))
            .expect("server thread hung or died")
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.tx.send(Request::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

// --- Server-side query/apply implementations ---------------------------------

fn answer(state: &AerowmState, outputs: &[smithay::output::Output], q: Query) -> Answer {
    match q {
        Query::Snapshot => Answer::Snapshot(state.snapshot()),
        Query::SurfaceCount => Answer::Count(state.surfaces.len()),
        Query::WorkspaceWindows(i) => Answer::Ids(
            state
                .workspaces
                .get(i)
                .map(|ws| ws.get_windows().iter().map(|id| id.as_usize()).collect())
                .unwrap_or_default(),
        ),
        Query::SurfaceApps => Answer::Apps(
            state
                .surfaces
                .keys()
                .map(|id| {
                    let app = aerowm::session::app_id_for_window(state, *id)
                        .map(|a| a.id)
                        .unwrap_or_default();
                    (id.as_usize(), app)
                })
                .collect(),
        ),
        Query::RuleCount => Answer::Count(state.engine.rule_count()),
        Query::EvalRules(class) => {
            Answer::Count(state.engine.evaluate_rules(&class, None).workspace.unwrap_or(0))
        }
        Query::FloatingIds => Answer::Ids(
            state
                .surfaces
                .keys()
                .filter(|id| state.workspaces.iter().any(|ws| ws.is_floating(**id)))
                .map(|id| id.as_usize())
                .collect(),
        ),
        Query::Focused => {
            Answer::OptId(state.active_workspace().get_focused().map(|id| id.as_usize()))
        }
        Query::Geometries => Answer::Geoms(
            state
                .surfaces
                .iter()
                .filter_map(|(id, surface)| {
                    let window = state
                        .space
                        .elements()
                        .find(|w| w.toplevel() == Some(surface))?;
                    let loc = state.space.element_location(window)?;
                    let size = window.geometry().size;
                    Some(Geom {
                        id: id.as_usize(),
                        x: loc.x,
                        y: loc.y,
                        w: size.w.max(0) as u32,
                        h: size.h.max(0) as u32,
                    })
                })
                .collect(),
        ),
        Query::OutputCount => Answer::Count(state.space.outputs().count()),
        Query::UsableArea(i) => {
            let area = outputs
                .get(i)
                .map(|o| state.usable_area_for_output(o))
                .expect("test requested a missing output");
            Answer::Area((area.origin.x, area.origin.y, area.size.width, area.size.height))
        }
    }
}

fn apply(
    dh: &wayland_server::DisplayHandle,
    _display: &mut Display<AerowmState>,
    state: &mut AerowmState,
    outputs: &mut Vec<smithay::output::Output>,
    a: Act,
) -> ActResult {
    match a {
        Act::LoadConfig(code) => match state.engine.load_config_string(&code) {
            Ok(()) => ActResult::Ok,
            Err(e) => ActResult::ConfigErr(e.to_string()),
        },
        Act::ToggleFloating => {
            state.toggle_floating();
            ActResult::Ok
        }
        Act::FocusNext => {
            state.focus_next();
            ActResult::Ok
        }
        Act::FocusPrev => {
            state.focus_prev();
            ActResult::Ok
        }
        Act::KillActive => {
            state.kill_active();
            ActResult::Ok
        }
        Act::SwitchWorkspace(i) => {
            state.switch_workspace(i);
            ActResult::Ok
        }
        Act::CleanupDead => {
            state.cleanup_dead_windows();
            ActResult::Ok
        }
        Act::ApplyLayout => {
            state.apply_layout();
            ActResult::Ok
        }
        Act::AddOutput { name, w, h } => {
            use smithay::output::{Mode, Output, PhysicalProperties};
            use smithay::utils::{Size, Transform};
            let output = Output::new(
                name.clone(),
                PhysicalProperties {
                    size: (0, 0).into(),
                    subpixel: smithay::output::Subpixel::Unknown,
                    make: "test".into(),
                    model: "headless".into(),
                },
            );
            let mode = Mode {
                size: Size::from((w, h)),
                refresh: 60_000,
            };
            output.change_current_state(Some(mode), Some(Transform::Normal), Some(smithay::output::Scale::Integer(1)), None);
            output.create_global::<AerowmState>(dh);
            state.space.map_output(&output, (0, 0));
            state.output = Some(output.clone());
            outputs.push(output);
            ActResult::Ok
        }
        #[cfg(feature = "xwayland")]
        Act::TryXwaylandSpawn => {
            // Real spawn against our display handle. Without the Xwayland
            // binary this must fail gracefully (the compositor keeps
            // running Wayland-only); the test only exercises that path
            // when the binary is absent.
            match smithay::xwayland::XWayland::spawn(
                dh,
                None,
                Vec::<(String, String)>::new(),
                true,
                std::process::Stdio::null(),
                std::process::Stdio::null(),
                |_| {},
            ) {
                Ok(_) => ActResult::XwaylandStarted,
                Err(e) => ActResult::XwaylandFailed(e.to_string()),
            }
        }
        #[cfg(not(feature = "xwayland"))]
        Act::TryXwaylandSpawn => ActResult::XwaylandFailed("xwayland feature disabled".into()),
    }
}

// --- Polling helper for scheduling races -------------------------------------

/// Retries `cond` until it holds or `timeout` elapses (server and client
/// run on different threads; an assertion right after `roundtrip` can win
/// the race against the 1ms pump).
pub fn wait_for(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if cond() {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    cond()
}
