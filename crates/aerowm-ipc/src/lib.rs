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
    /// Toggle scratchpad workspace
    ToggleScratchpad,
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
    pub layout: String,
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

/// Hard cap for a single IPC request line. Commands are tiny JSON
/// objects; anything larger is a bug or abuse — reject it instead of
/// buffering unboundedly.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;

/// Reads one newline-delimited JSON frame from `stream`.
///
/// Stream sockets may split or coalesce writes, so this loops until the
/// first `\n` (or EOF) instead of trusting a single `read`. Returns the
/// first line without the terminator; any bytes pipelined after it do not
/// belong to this protocol (one connection = one command) and are
/// discarded. Returns `Ok(None)` on clean EOF with no data, `Err` on
/// I/O errors or oversized frames. The stream is put in blocking mode
/// for the duration of the read.
pub fn read_frame(
    stream: &mut std::os::unix::net::UnixStream,
) -> std::io::Result<Option<String>> {
    use std::io::Read;
    stream.set_nonblocking(false)?;
    let mut buf = Vec::with_capacity(256);
    let mut tmp = [0u8; 1024];
    loop {
        if buf.len() > MAX_FRAME_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "IPC frame exceeds size limit",
            ));
        }
        match stream.read(&mut tmp) {
            Ok(0) => break, // EOF: parse what we got
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if buf.contains(&b'\n') {
                    break;
                }
            }
            Err(e) => return Err(e),
        }
    }
    if buf.is_empty() {
        return Ok(None);
    }
    let end = buf.iter().position(|&b| b == b'\n').unwrap_or(buf.len());
    buf.truncate(end);
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}
