use aerowm_lua::ScriptEngine;
use wayland_server::DisplayHandle;
use std::os::unix::net::UnixStream;
use std::io::Write;
use aerowm_ipc::IpcEvent;

pub struct AerowmState {
    pub engine: ScriptEngine,
    pub is_running: bool,
    pub subscribers: Vec<UnixStream>,
    // Note: Smithay compositor states (CompositorState, ShmState, XdgShellState, etc.)
    // will be added here. They require implementing extensive trait delegates 
    // (`delegate_compositor!`, `delegate_shm!`, etc.) which we will build iteratively.
}

impl AerowmState {
    pub fn new(_display_handle: &DisplayHandle, engine: ScriptEngine) -> Self {
        Self {
            engine,
            is_running: true,
            subscribers: Vec::new(),
        }
    }

    /// Broadcasts a live event to all connected Pub/Sub IPC clients (e.g. status bars).
    pub fn broadcast_event(&mut self, event: IpcEvent) {
        if self.subscribers.is_empty() {
            return;
        }
        if let Ok(mut payload) = serde_json::to_string(&event) {
            payload.push('\n'); // newline delimited JSON
            
            // Send payload, removing any subscriber that disconnected (write failed)
            self.subscribers.retain_mut(|stream| {
                stream.write_all(payload.as_bytes()).is_ok()
            });
        }
    }
}
