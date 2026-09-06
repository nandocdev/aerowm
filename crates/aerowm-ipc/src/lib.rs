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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkspaceAction {
    Next,
    Prev,
    Switch(usize),
}

/// The response sent back from aerowm to aerowm-ctl
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcResponse {
    pub success: bool,
    pub message: Option<String>,
}

pub const IPC_SOCKET_PATH: &str = "/tmp/aerowm.sock";
