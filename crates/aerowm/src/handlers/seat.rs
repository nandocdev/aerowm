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
    
    fn focus_changed(&mut self, _seat: &smithay::input::Seat<Self>, _focused: Option<&WlSurface>) {
        // Handle keyboard focus changes
    }
    
    fn cursor_image(&mut self, _seat: &smithay::input::Seat<Self>, _image: CursorImageStatus) {
        // Handle cursor image changes requested by clients
    }
}

delegate_seat!(AerowmState);
