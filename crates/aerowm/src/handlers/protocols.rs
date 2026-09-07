use smithay::{
    delegate_fractional_scale, delegate_viewporter, delegate_xdg_decoration,
};
use smithay::wayland::fractional_scale::FractionalScaleHandler;
use smithay::wayland::shell::xdg::decoration::XdgDecorationHandler;
use smithay::wayland::shell::xdg::ToplevelSurface;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
use crate::state::AerowmState;

impl FractionalScaleHandler for AerowmState {
    fn new_fractional_scale(&mut self, _surface: WlSurface) {}
}

impl XdgDecorationHandler for AerowmState {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(Mode::ClientSide);
        });
        toplevel.send_configure();
    }
    
    fn request_mode(&mut self, toplevel: ToplevelSurface, _mode: Mode) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(Mode::ClientSide);
        });
        toplevel.send_configure();
    }
    
    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(Mode::ClientSide);
        });
        toplevel.send_configure();
    }
}

delegate_fractional_scale!(AerowmState);
delegate_viewporter!(AerowmState);
delegate_xdg_decoration!(AerowmState);

