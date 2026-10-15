use smithay::{
    delegate_session_lock,
    reexports::wayland_server::protocol::wl_output::WlOutput,
    wayland::session_lock::{
        LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker,
    },
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
            keyboard.set_focus(
                self,
                None::<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
                0.into(),
            );
        }
    }

    fn unlock(&mut self) {
        self.is_locked = false;
        self.lock_surfaces.clear();
        // Typically we drop all lock surfaces here or let the client destroy them
    }

    fn new_surface(&mut self, surface: LockSurface, output: WlOutput) {
        let size = smithay::output::Output::from_resource(&output)
            .and_then(|o| o.current_mode())
            .map(|m| (m.size.w.max(0) as u32, m.size.h.max(0) as u32).into())
            .or_else(|| self.output_size.map(|s| (s.w.max(0) as u32, s.h.max(0) as u32).into()))
            .unwrap_or_else(|| (1920u32, 1080u32).into());

        // Send initial configure
        surface.with_pending_state(|state| {
            state.size = Some(size);
        });
        surface.send_configure();

        // Give keyboard focus to the lock surface
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, Some(surface.wl_surface().clone()), 0.into());
        }

        self.lock_surfaces.push(surface);
    }
}

delegate_session_lock!(AerowmState);
