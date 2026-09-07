use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
};
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::delegate_xdg_shell;
use smithay::desktop::Window;
use smithay::desktop::PopupKind;

use crate::state::AerowmState;

impl XdgShellHandler for AerowmState {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let id = aerowm_core::id::WindowId::new();
        let prev_focus = self.active_workspace().get_focused();
        let app = crate::session::app_id_of_toplevel(&surface);

        // Register in the domain layout
        self.active_workspace_mut().add_window(id);
        self.surfaces.insert(id, surface.clone());

        // Create a Window from the toplevel surface
        let window = Window::new_wayland_window(surface.clone());

        // Add to space at (0, 0) - layout will be applied later
        self.space.map_element(window, (0, 0), true);

        // Restored session placement (no-op unless a pending entry matches).
        if let Some(app) = app {
            self.apply_pending_placement(id, &app, prev_focus);
        }

        // Apply layout to position all windows
        self.apply_layout();
        self.update_keyboard_focus();

        // Broadcast window opened event
        self.broadcast_event(aerowm_ipc::IpcEvent::WindowOpened(id.as_usize()));

        surface.with_pending_state(|state| {
            state.states.set(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Activated);
        });
        surface.send_configure();
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        // Handle popups (tooltips, context menus)
        let popup_kind: PopupKind = surface.into();
        self.popup_manager.track_popup(popup_kind).ok();
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: smithay::utils::Serial) {
        // Handle grab for popups - simplified for now
    }

    fn reposition_request(
        &mut self,
        _surface: PopupSurface,
        _positioner: PositionerState,
        _token: u32,
    ) {
        // Handle popup reposition - simplified for now
    }
}

// XdgShell requires a companion ClientState which we will build into our global ClientState
delegate_xdg_shell!(AerowmState);
