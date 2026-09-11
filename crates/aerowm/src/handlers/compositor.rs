use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::Client;
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{CompositorClientState, CompositorHandler, CompositorState};
use smithay::wayland::shm::{ShmHandler, ShmState};
use smithay::{delegate_compositor, delegate_output, delegate_shm};

use crate::state::AerowmState;

pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

impl Default for ClientState {
    fn default() -> Self {
        Self {
            compositor_state: CompositorClientState::default(),
        }
    }
}

impl CompositorHandler for AerowmState {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        // Retrieve the ClientState attached to this client.
        if let Some(state) = client.get_data::<ClientState>() {
            &state.compositor_state
        } else {
            // The internal XWayland client carries smithay's own client
            // data instead of ours; serve its compositor state directly.
            #[cfg(feature = "xwayland")]
            if let Some(xstate) = client.get_data::<smithay::xwayland::XWaylandClientData>() {
                return &xstate.compositor_state;
            }
            // Fallback for cases where ClientData isn't fully set up yet
            panic!("ClientState not attached to Client")
        }
    }

    fn commit(&mut self, surface: &WlSurface) {
        use smithay::backend::renderer::utils::on_commit_buffer_handler;
        use smithay::desktop::layer_map_for_output;

        // Buffer/damage bookkeeping for every surface (windows and layers).
        on_commit_buffer_handler::<Self>(surface);

        // Surface committed: call on_commit on all windows to update damage tracking
        for window in self.space.elements() {
            window.on_commit();
        }
        self.popup_manager.commit(surface);

        // If a layer surface changed (size, anchor, margins, exclusive
        // zone), re-arrange its output map and re-tile around it.
        let outputs: Vec<_> = self.space.outputs().cloned().collect();
        for output in &outputs {
            let changed = {
                let mut map = layer_map_for_output(output);
                let is_layer = map.layers().any(|l| l.wl_surface() == surface);
                if is_layer {
                    map.arrange()
                } else {
                    false
                }
            };
            if changed {
                self.apply_layout();
                break;
            }
        }
    }
}

impl BufferHandler for AerowmState {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl ShmHandler for AerowmState {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

delegate_compositor!(AerowmState);
delegate_shm!(AerowmState);
// Routes wl_output/xdg_output binds through OutputManagerState, which
// initializes every new instance (geometry, modes, scale, done).
delegate_output!(AerowmState);
