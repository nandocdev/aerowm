use serde::{Deserialize, Serialize};

/// IPC Commands sent from aerowm-ctl to the main aerowm process
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcCommand {
    /// Reload the configuration file (Luau)
    ReloadConfig,
    /// Switch to a different workspace
    Workspace { action: WorkspaceAction },
    /// Kill the currently focused window
    KillWindow,
    /// Stop the compositor
    Exit,
    /// Hot restart: dump `session.json`, then re-exec the compositor
    /// inheriting the Wayland listening socket. Layout state is restored;
    /// already-connected clients are reaped by the exec itself.
    Restart,
    /// Subscribe to live events stream (Pub/Sub)
    Subscribe,
    /// Query a full state snapshot (workspaces, focus). Used by `aerowm-bar`.
    /// The snapshot is returned as JSON inside [`IpcResponse::message`].
    GetState,
}

/// Point-in-time view of the compositor, polled by status clients.
///
/// Field additions are backward compatible as long as old fields keep
/// their names and types (serde ignores unknown fields on parse).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompositorSnapshot {
    /// One entry per workspace, in index order.
    pub workspaces: Vec<WorkspaceInfo>,
    /// Index into [`CompositorSnapshot::workspaces`] of the active workspace.
    pub active: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkspaceInfo {
    /// User-facing name ("1", "2", ...).
    pub name: String,
    /// `true` when this is the active workspace.
    pub active: bool,
    /// Number of tiled windows on this workspace.
    pub window_count: usize,
    /// Window id of the focused window, if any.
    pub focused_window: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkspaceAction {
    Next,
    Prev,
    Switch(usize),
}

/// The response sent back from aerowm to aerowm-ctl for single-shot commands
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcResponse {
    pub success: bool,
    pub message: Option<String>,
}

/// Real-time events broadcasted to all subscribed clients
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcEvent {
    WorkspaceSwitched(String),
    FocusChanged(Option<usize>),
    WindowOpened(usize),
    WindowClosed(usize),
}

/// Computes the IPC socket path at runtime.
///
/// Prefers `$XDG_RUNTIME_DIR/aerowm.sock` (per-user, `tmpfs`, cleaned up
/// on logout) and falls back to `/tmp/aerowm-<uid>.sock` when the variable
/// is unset (e.g. minimal environments). The uid suffix keeps multiple
/// local users from colliding on, or hijacking, each other's socket —
/// important because the socket exposes privileged commands
/// (`spawn` via config, `kill`, `restart`, `exit`).
pub fn ipc_socket_path() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR")
        && !dir.is_empty()
    {
        return std::path::PathBuf::from(dir).join("aerowm.sock");
    }
    #[cfg(unix)]
    {
        // Uid-suffixed so users never share a socket path.
        let uid = get_euid().unwrap_or(0);
        std::path::PathBuf::from(format!("/tmp/aerowm-{uid}.sock"))
    }
    #[cfg(not(unix))]
    return std::path::PathBuf::from("/tmp/aerowm.sock");
}

#[cfg(unix)]
fn get_euid() -> Option<u32> {
    // No libc dependency in this crate: parse `/proc/self/status`.
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|l| l.strip_prefix("Uid:"))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|s| s.parse().ok())
}
