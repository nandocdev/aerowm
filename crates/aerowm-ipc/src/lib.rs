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
