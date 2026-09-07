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

pub const IPC_SOCKET_PATH: &str = "/tmp/aerowm.sock";
