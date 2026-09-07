use calloop::{generic::Generic, Interest, Mode, LoopHandle, PostAction};
use std::os::unix::net::UnixListener;
use std::io::Write;
use tracing::{info, warn, error};
use aerowm_ipc::{IpcCommand, IpcResponse, WorkspaceAction, ipc_socket_path, read_frame};
use crate::state::AerowmState;

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
                // One newline-delimited JSON frame per connection; see
                // `aerowm_ipc::read_frame` for the reassembly contract.
                let payload = match read_frame(&mut stream) {
                    Ok(Some(line)) => line,
                    Ok(None) => return Ok(PostAction::Continue), // clean EOF
                    Err(e) => {
                        warn!("IPC frame error: {e}");
                        return Ok(PostAction::Continue);
                    }
                };
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
