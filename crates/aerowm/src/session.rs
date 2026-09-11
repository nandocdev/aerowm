//! Session persistence and in-place hot restart.
//!
//! On `aerowm-ctl restart` the compositor dumps [`SessionState`] to
//! `session.json`, then re-execs itself:
//!
//! * the Wayland **listening** socket FD is inherited (CLOEXEC cleared),
//!   so `WAYLAND_DISPLAY` stays valid and never flaps;
//! * the new process restores workspaces, layouts, stacking order, focus
//!   and floating geometry, then re-places windows by app identity as
//!   clients (re)connect.
//!
//! Honest limitation: `exec` replaces the process image, so already-
//! connected clients lose their protocol state and are reaped by the
//! kernel/toolkit. What survives losslessly is the listening socket and
//! the full WM layout state — relaunched apps tile exactly as before.

use std::os::unix::io::RawFd;
use std::path::PathBuf;

use aerowm_core::session::{AppId, PendingPlacement, SessionState, WorkspaceSnapshot};
use aerowm_core::workspace::Workspace;

use crate::state::AerowmState;

/// Env var: raw FD number of the inherited Wayland listener (set by us).
pub use crate::wayland_socket::WAYLAND_FD_ENV;
/// Env var: marks a boot as the child of an in-place restart.
pub const RESTARTED_ENV: &str = "AEROWM_RESTARTED";
/// Env var: explicit session file path (defaults to [`default_session_path`]).
pub const SESSION_FILE_ENV: &str = "AEROWM_SESSION_FILE";

/// `session.json` lives next to the sockets: it is a restart-handoff file,
/// not long-term config.
pub fn default_session_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    dir.join("aerowm-session.json")
}

pub fn session_file_path() -> PathBuf {
    std::env::var_os(SESSION_FILE_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(default_session_path)
}

/// Builds a [`SessionState`] from live compositor state.
pub fn build_session(state: &AerowmState) -> SessionState {
    let workspaces = state
        .workspaces
        .iter()
        .map(|ws| {
            let focused_id = ws.get_focused();
            // Windows without usable identity cannot be matched back after
            // a restart, so they are left out of the session file.
            let windows = ws
                .get_windows()
                .iter()
                .filter_map(|id| {
                    app_id_for_window(state, *id).map(|app| aerowm_core::session::WindowEntry {
                        app,
                        floating: ws.is_floating(*id),
                        geo: state.float_geo.get(id).copied(),
                        focused: Some(*id) == focused_id,
                    })
                })
                .collect();
            WorkspaceSnapshot {
                name: ws.name.clone(),
                layout: ws.layout_spec(),
                windows,
            }
        })
        .collect();
    SessionState {
        version: aerowm_core::session::SESSION_VERSION,
        active_workspace: state.active_ws,
        workspaces,
    }
}

/// App identity of a Wayland toplevel, if it carries a usable app_id.
pub fn app_id_of_toplevel(
    surface: &smithay::wayland::shell::xdg::ToplevelSurface,
) -> Option<AppId> {
    let (app_id, title) =
        smithay::wayland::compositor::with_states(surface.wl_surface(), |states| {
            let data = states
                .data_map
                .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>();
            match data {
                Some(data) => {
                    let data = data.lock().unwrap();
                    (data.app_id.clone(), data.title.clone())
                }
                None => (None, None),
            }
        });
    app_id
        .filter(|s| !s.is_empty())
        .map(|app_id| AppId::wayland(app_id, title))
}

/// App identity of an X11 surface (class, falling back to instance).
#[cfg(feature = "xwayland")]
pub fn app_id_of_x11(surface: &smithay::xwayland::X11Surface) -> Option<AppId> {
    let class = surface.class();
    let ident = if class.is_empty() {
        surface.instance()
    } else {
        class
    };
    if ident.is_empty() {
        return None;
    }
    let title = {
        let t = surface.title();
        if t.is_empty() {
            None
        } else {
            Some(t)
        }
    };
    Some(AppId::x11(ident, title))
}

/// Best-effort app identity for a managed window id.
///
/// Returns `None` when the window carries no usable identifier: such
/// windows must never participate in placement matching, otherwise two
/// unidentified windows would steal each other's slots.
pub fn app_id_for_window(state: &AerowmState, id: aerowm_core::id::WindowId) -> Option<AppId> {
    if let Some(toplevel) = state.surfaces.get(&id) {
        return app_id_of_toplevel(toplevel);
    }
    #[cfg(feature = "xwayland")]
    if let Some(x11) = state.x11_surfaces.get(&id) {
        return app_id_of_x11(x11);
    }
    None
}

/// Writes `session.json` atomically (temp file + rename).
pub fn dump_session(state: &AerowmState) -> Result<PathBuf, String> {
    let session = build_session(state);
    let json = session
        .to_json()
        .map_err(|e| format!("serializing session: {e}"))?;
    let path = session_file_path();
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| format!("writing {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("renaming {}: {e}", path.display()))?;
    tracing::info!("session dumped to {}", path.display());
    Ok(path)
}

/// Restores workspace scaffolding + pending placements from `session.json`.
/// Windows themselves arrive later and are placed by [`AerowmState::apply_pending_placement`].
pub fn restore_session(state: &mut AerowmState) -> Result<usize, String> {
    let path = session_file_path();
    let raw =
        std::fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let session = SessionState::from_json(&raw).map_err(|e| format!("session: {e}"))?;

    state.workspaces = session
        .workspaces
        .iter()
        .map(|snap| {
            let mut ws = Workspace::new(&snap.name);
            ws.set_layout(snap.layout.clone());
            ws
        })
        .collect();
    state.active_ws = session
        .active_workspace
        .min(state.workspaces.len().saturating_sub(1));
    state.pending_placements = session.pending_placements();

    // Focus flags ride along with placements; the active workspace itself
    // is restored immediately so the bar is correct from the first frame.
    tracing::info!(
        "session restored from {}: {} workspaces, {} pending windows",
        path.display(),
        state.workspaces.len(),
        state.pending_placements.len()
    );
    Ok(state.pending_placements.len())
}

/// Makes the Wayland listener inheritable and re-execs the current
/// binary. Only returns on failure (the session must already be dumped).
///
/// `keep_display` must be true when running with the nested winit backend:
/// `DISPLAY` then names the *host* X server, which outlives the exec and is
/// needed to recreate the nesting window. Otherwise `DISPLAY` may point at
/// our own pre-exec XWayland, which dies with us (the fresh boot re-exports
/// it when its own XWayland is ready), so it is dropped.
pub fn exec_restart(
    wl_fd: RawFd,
    session_path: &std::path::Path,
    keep_display: bool,
) -> Result<(), String> {
    // The listener must survive exec: std sockets are CLOEXEC by default.
    // SAFETY: `wl_fd` is our own open listening socket (recorded at bind
    // time from `AsRawFd`), so borrowing it raw here is sound.
    let borrowed = unsafe { std::os::unix::io::BorrowedFd::borrow_raw(wl_fd) };
    rustix::io::fcntl_setfd(borrowed, rustix::io::FdFlags::empty())
        .map_err(|e| format!("clearing CLOEXEC on wayland socket: {e}"))?;

    let exe = std::env::current_exe().map_err(|e| format!("resolving current exe: {e}"))?;
    let mut cmd = std::process::Command::new(exe);
    // Forward our CLI args verbatim (backend selection, …).
    cmd.args(std::env::args_os().skip(1));
    cmd.env(WAYLAND_FD_ENV, wl_fd.to_string());
    cmd.env(RESTARTED_ENV, "1");
    cmd.env(SESSION_FILE_ENV, session_path);
    if !keep_display {
        cmd.env_remove("DISPLAY");
    }

    tracing::info!("hot-restarting via exec (wayland fd={wl_fd})…");
    use std::os::unix::process::CommandExt as _;
    let _ = cmd.exec();
    Err("exec returned unexpectedly".into())
}

/// Removes and returns the first pending placement matching `app`
/// (same backend kind and identifier), or `None` when nothing matches.
pub fn consume_placement(
    pending: &mut Vec<PendingPlacement>,
    app: &AppId,
) -> Option<PendingPlacement> {
    let pos = pending
        .iter()
        .position(|p| p.app.kind == app.kind && p.app.id == app.id)?;
    Some(pending.remove(pos))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aerowm_core::session::{AppKind, WorkspaceSnapshot};

    /// SESSION_FILE_ENV is process-global; serialize mutating tests.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn test_state() -> AerowmState {
        let display = wayland_server::Display::<AerowmState>::new().expect("headless display");
        let engine = aerowm_lua::ScriptEngine::new().expect("lua engine");
        AerowmState::new(&display.handle(), engine)
    }

    #[test]
    fn dump_empty_state_roundtrips() {
        let _guard = lock_env();
        let state = test_state();
        let session = build_session(&state);
        assert_eq!(session.workspaces.len(), state.workspaces.len());
        assert!(session.workspaces.iter().all(|w| w.windows.is_empty()));

        // write + read back through a temp session file
        let dir = std::env::temp_dir().join(format!("aerowm-test-dump-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: test-only env mutation, restored below.
        unsafe {
            std::env::set_var(SESSION_FILE_ENV, dir.join("session.json").as_os_str());
        }
        let path = dump_session(&state).expect("dump");
        assert!(path.exists());

        let mut restored = test_state();
        // scribble to prove restore overwrites
        restored.active_ws = 3;
        let pending = restore_session(&mut restored).expect("restore");
        assert_eq!(pending, 0);
        assert_eq!(restored.active_ws, 0);
        assert_eq!(restored.workspaces.len(), state.workspaces.len());
        assert!(restored.pending_placements.is_empty());

        unsafe {
            std::env::remove_var(SESSION_FILE_ENV);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_rebuilds_layouts_and_pending() {
        let _guard = lock_env();
        let snapshots = vec![
            WorkspaceSnapshot {
                name: "code".into(),
                layout: aerowm_core::layout::LayoutSpec::Columns,
                windows: vec![],
            },
            WorkspaceSnapshot {
                name: "web".into(),
                layout: aerowm_core::layout::LayoutSpec::Max,
                windows: vec![],
            },
        ];
        let session = SessionState {
            version: aerowm_core::session::SESSION_VERSION,
            active_workspace: 1,
            workspaces: snapshots,
        };
        // one pending entry waiting for a reconnecting client
        let mut session = session;
        session.workspaces[0]
            .windows
            .push(aerowm_core::session::WindowEntry {
                app: AppId::wayland("kitty", None),
                floating: true,
                geo: Some(aerowm_core::geometry::Rect::new(10, 20, 300, 400)),
                focused: true,
            });

        let dir = std::env::temp_dir().join(format!("aerowm-test-restore-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.json");
        std::fs::write(&path, session.to_json().unwrap()).unwrap();
        unsafe {
            std::env::set_var(SESSION_FILE_ENV, path.as_os_str());
        }

        let mut state = test_state();
        let pending = restore_session(&mut state).expect("restore");
        assert_eq!(pending, 1);
        assert_eq!(state.active_ws, 1);
        assert_eq!(state.workspaces[0].name, "code");
        assert_eq!(
            state.workspaces[0].layout_spec(),
            aerowm_core::layout::LayoutSpec::Columns
        );
        assert_eq!(
            state.workspaces[1].layout_spec(),
            aerowm_core::layout::LayoutSpec::Max
        );
        assert_eq!(state.pending_placements.len(), 1);
        assert_eq!(state.pending_placements[0].workspace, 0);
        assert!(state.pending_placements[0].focused);

        unsafe {
            std::env::remove_var(SESSION_FILE_ENV);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn consume_placement_matches_in_order() {
        let mk = |id: &str| PendingPlacement {
            app: AppId::wayland(id, None),
            workspace: 0,
            position: 0,
            floating: false,
            geo: None,
            focused: false,
        };
        let mut pending = vec![mk("a"), mk("b"), mk("a")];
        let first = consume_placement(&mut pending, &AppId::wayland("a", None)).unwrap();
        assert_eq!(first.app.id, "a");
        assert_eq!(pending.len(), 2);
        // same-kind+id required: X11 class "a" must not match Wayland "a"
        let x11 = AppId {
            kind: AppKind::X11,
            id: "a".into(),
            title: None,
        };
        assert!(consume_placement(&mut pending, &x11).is_none());
        assert_eq!(pending.len(), 2);
        // second Wayland "a" still queued behind "b"
        let second = consume_placement(&mut pending, &AppId::wayland("a", None)).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].app.id, "b");
        let _ = second;
    }
}
