use smithay::{
    delegate_session_lock,
    wayland::session_lock::{
        LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker,
    },
    reexports::wayland_server::protocol::wl_output::WlOutput,
};

use crate::state::AerowmState;

impl SessionLockHandler for AerowmState {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.session_lock_state
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        if self.is_locked {
            return;
        }
        self.is_locked = true;
        confirmation.lock();
    }

    fn unlock(&mut self) {
        self.is_locked = false;
        // Typically we drop all lock surfaces here or let the client destroy them
    }

    fn new_surface(&mut self, surface: LockSurface, _output: WlOutput) {
        // Send initial configure
        surface.with_pending_state(|state| {
            state.size = Some((1920, 1080).into()); // Hardcoded bounds for now
        });
        surface.send_configure();
    }
}

delegate_session_lock!(AerowmState);
