use smithay::input::{SeatHandler, SeatState, pointer::CursorImageStatus};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::delegate_seat;
use crate::state::AerowmState;

impl SeatHandler for AerowmState {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<AerowmState> {
        &mut self.seat_state
    }
    
    fn focus_changed(&mut self, seat: &smithay::input::Seat<Self>, focused: Option<&WlSurface>) {
        // Keep the winit window title in sync-ish and log for debugging;
        // real focus state lives in the active Workspace + keyboard focus.
        tracing::debug!(
            seat = seat.name(),
            has_focus = focused.is_some(),
            "keyboard focus changed"
        );
    }

    fn cursor_image(&mut self, _seat: &smithay::input::Seat<Self>, _image: CursorImageStatus) {
        // Nested winit backend: the host cursor is used, nothing to upload.
    }
}

delegate_seat!(AerowmState);
