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
        
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, None::<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>, 0.into());
        }
    }

    fn unlock(&mut self) {
        self.is_locked = false;
        self.lock_surfaces.clear();
        // Typically we drop all lock surfaces here or let the client destroy them
    }

    fn new_surface(&mut self, surface: LockSurface, _output: WlOutput) {
        // Send initial configure
        surface.with_pending_state(|state| {
            state.size = Some((1920, 1080).into()); // Hardcoded bounds for now
        });
        surface.send_configure();
        
        // Give keyboard focus to the lock surface
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, Some(surface.wl_surface().clone().into()), 0.into());
        }
        
        self.lock_surfaces.push(surface);
    }
}

delegate_session_lock!(AerowmState);
