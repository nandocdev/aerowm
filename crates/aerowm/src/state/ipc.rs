//! State for status clients: the snapshot answered to `aerowm-ctl
//! GetState` and the event stream pushed to subscribers.

use std::io::Write;

use aerowm_ipc::IpcEvent;

use super::AerowmState;

impl AerowmState {
    /// Builds a serializable snapshot for status clients (`aerowm-bar`).
    pub fn snapshot(&self) -> aerowm_ipc::CompositorSnapshot {
        let workspaces = self
            .workspaces
            .iter()
            .enumerate()
            .map(|(idx, ws)| aerowm_ipc::WorkspaceInfo {
                name: ws.name.clone(),
                active: idx == self.active_ws,
                window_count: ws.get_windows().len(),
                focused_window: ws.get_focused().map(|id| id.as_usize()),
            })
            .collect();
        aerowm_ipc::CompositorSnapshot {
            workspaces,
            active: self.active_ws,
            layout: self
                .active_workspace()
                .get_current_layout()
                .name()
                .to_string(),
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
            self.subscribers
                .retain_mut(|stream| stream.write_all(payload.as_bytes()).is_ok());
        }
    }
}
