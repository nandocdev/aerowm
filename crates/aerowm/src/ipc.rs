use calloop::{generic::Generic, Interest, Mode, LoopHandle, PostAction};
use std::os::unix::net::UnixListener;
use std::io::{Read, Write};
use tracing::{info, warn, error};
use aerowm_ipc::{IpcCommand, IpcResponse, WorkspaceAction, ipc_socket_path};
use crate::state::AerowmState;

/// Hard cap for a single IPC request line. Commands are tiny JSON
/// objects; anything larger is a bug or abuse — drop it instead of
/// buffering unboundedly.
const MAX_IPC_LINE: usize = 64 * 1024;

pub fn init_ipc_socket(
    loop_handle: LoopHandle<'_, AerowmState>,
) -> Result<(), Box<dyn std::error::Error>> {
    let socket_path = ipc_socket_path();

    // Cleanup old socket if compositor crashed previously
    if socket_path.exists() {
        std::fs::remove_file(&socket_path)?;
    }

    let listener = UnixListener::bind(&socket_path)?;
    // Owner-only: the socket exposes kill/restart/exit, so no other
    // local user (or their processes) may connect to it.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;
    }
    listener.set_nonblocking(true)?;

    info!("Listening for IPC on {:?}", socket_path);

    // Watch the unix listener socket for read events
    let source = Generic::new(listener, Interest::READ, Mode::Level);

    loop_handle.insert_source(source, |_, listener, state| {
        match listener.accept() {
            Ok((mut stream, _)) => {
                // Frame the request as one newline-delimited JSON line.
                // Clients (aerowm-ctl, aerowm-bar) always terminate with
                // `\n`; read until it (or EOF) instead of trusting a
                // single `read` to deliver the whole datagram — stream
                // sockets may split or coalesce writes, and payloads can
                // exceed any fixed buffer.
                if stream.set_nonblocking(false).is_err() {
                    return Ok(PostAction::Continue);
                }
                let mut line = Vec::with_capacity(256);
                let mut tmp = [0u8; 1024];
                let framed = loop {
                    if line.len() > MAX_IPC_LINE {
                        warn!("IPC payload exceeds {MAX_IPC_LINE} bytes, dropping");
                        break None;
                    }
                    match stream.read(&mut tmp) {
                        Ok(0) => break Some(()), // EOF: parse what we got
                        Ok(n) => {
                            line.extend_from_slice(&tmp[..n]);
                            if line.contains(&b'\n') {
                                break Some(());
                            }
                        }
                        Err(e) => {
                            warn!("IPC read error: {e}");
                            break None;
                        }
                    }
                };
                if framed.is_none() {
                    return Ok(PostAction::Continue);
                }
                // First line only; anything pipelined after it is not part
                // of this protocol (one connection = one command).
                let payload = String::from_utf8_lossy(&line);
                let payload = payload.lines().next().unwrap_or("");
                // Parse command
                if let Ok(command) = serde_json::from_str::<IpcCommand>(payload.trim()) {
                    // `GetState` answers with a snapshot instead of a plain ACK.
                    if matches!(command, IpcCommand::GetState) {
                        let snapshot = state.snapshot();
                        let response = IpcResponse {
                            success: true,
                            message: serde_json::to_string(&snapshot).ok(),
                        };
                        let resp_str =
                            serde_json::to_string(&response).unwrap_or_default();
                        let _ = stream.write_all(resp_str.as_bytes());
                        return Ok(PostAction::Continue);
                    }
                    match command {
                        IpcCommand::GetState => unreachable!("handled above"),
                        IpcCommand::Subscribe => {
                            info!("New IPC subscriber connected");
                            stream.set_nonblocking(true).ok();
                            state.subscribers.push(stream);
                            return Ok(PostAction::Continue);
                        }
                        IpcCommand::ReloadConfig => {
                            info!("IPC: ReloadConfig requested");
                            if let Err(e) = state.reload_config() {
                                error!("Config reload failed: {}", e);
                            }
                        }
                        IpcCommand::Exit => {
                            info!("IPC: Exit requested");
                            state.is_running = false;
                        }
                        IpcCommand::Restart => {
                            info!("IPC: Restart requested");
                            state.restart_requested = true;
                            state.is_running = false;
                        }
                        IpcCommand::KillWindow => {
                            info!("IPC: KillWindow requested");
                            state.kill_active();
                        }
                        IpcCommand::Workspace { action } => {
                            info!("IPC: Workspace action: {:?}", action);
                            match action {
                                WorkspaceAction::Next => state.next_workspace(),
                                WorkspaceAction::Prev => state.prev_workspace(),
                                WorkspaceAction::Switch(i) => {
                                    // User-facing workspaces are 1-based ("1".."4").
                                    let idx = if i == 0 { 0 } else { i - 1 };
                                    state.switch_workspace(idx)
                                }
                            }
                        }
                    }

                    // Send standard ACK for single-shot commands
                    let response = IpcResponse { success: true, message: None };
                    let resp_str = serde_json::to_string(&response).unwrap_or_default();
                    let _ = stream.write_all(resp_str.as_bytes());
                } else {
                    warn!("IPC received malformed payload");
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // Expected in non-blocking mode, no-op
            }
            Err(e) => {
                error!("IPC Accept error: {}", e);
            }
        }
        Ok(PostAction::Continue)
    }).map_err(|e| format!("Failed to insert IPC source into calloop: {}", e))?;

    Ok(())
}
