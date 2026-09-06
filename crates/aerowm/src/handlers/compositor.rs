use smithay::wayland::compositor::{CompositorHandler, CompositorState, CompositorClientState};
use smithay::wayland::shm::{ShmHandler, ShmState};
use smithay::wayland::buffer::BufferHandler;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::Client;
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use smithay::{delegate_compositor, delegate_shm};


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
            // Fallback for cases where ClientData isn't fully set up yet
            panic!("ClientState not attached to Client")
        }
    }
    
    fn commit(&mut self, _surface: &WlSurface) {
        // Surface committed: call on_commit on all windows to update damage tracking
        for window in self.space.elements() {
            window.on_commit();
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
