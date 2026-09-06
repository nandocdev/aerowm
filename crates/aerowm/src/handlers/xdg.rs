use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
};
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::delegate_xdg_shell;

use crate::state::AerowmState;

impl XdgShellHandler for AerowmState {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let id = aerowm_core::id::WindowId::new();
        
        // Register in the domain layout
        self.active_workspace.add_window(id);
        self.surfaces.insert(id, surface.clone());

        // We use IpcEvent WindowOpened. WindowId can be serialized to u64 or string.
        // Wait, WindowId doesn't have an into() u64 directly in aerowm_core unless we implemented it.
        // I will just broadcast focus changed for now.
        // self.broadcast_event(aerowm_ipc::IpcEvent::FocusChanged(Some(1)));

        surface.with_pending_state(|state| {
            state.states.set(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Activated);
        });
        surface.send_configure();
    }

    fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {
        // Handle popups (tooltips, context menus)
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: smithay::utils::Serial) {
        // Handle grab for popups
    }

    fn reposition_request(
        &mut self,
        _surface: PopupSurface,
        _positioner: PositionerState,
        _token: u32,
    ) {
        // Handle popup reposition
    }
}

// XdgShell requires a companion ClientState which we will build into our global ClientState
delegate_xdg_shell!(AerowmState);
