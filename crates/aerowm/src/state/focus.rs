//! Keyboard focus, window activation and workspace navigation.
//!
//! Activation runs on workspace ids, so Wayland and X11 windows share
//! one path: the focused id decides, not the backend.

use aerowm_core::id::WindowId;
use aerowm_ipc::IpcEvent;
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::shell::xdg::ToplevelSurface;

#[cfg(feature = "xwayland")]
use smithay::xwayland::X11Surface;

use super::AerowmState;

impl AerowmState {
    /// Returns the wl_surface of the currently focused window, if any.
    pub fn focused_surface(&self) -> Option<ToplevelSurface> {
        self.active_workspace()
            .get_focused()
            .and_then(|id| self.surfaces.get(&id).cloned())
    }

    /// Focused X11 surface, if the focused window is an X11 one.
    #[cfg(feature = "xwayland")]
    pub fn focused_x11_surface(&self) -> Option<X11Surface> {
        self.active_workspace()
            .get_focused()
            .and_then(|id| self.x11_surfaces.get(&id).cloned())
    }

    /// `wl_surface` of the focused window, whether Wayland or X11.
    /// `None` when nothing (committed yet) is focused.
    pub fn focused_wl_surface(
        &self,
    ) -> Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface> {
        if let Some(surface) = self.focused_surface() {
            return Some(surface.wl_surface().clone());
        }
        #[cfg(feature = "xwayland")]
        if let Some(x11) = self.focused_x11_surface() {
            return x11.wl_surface();
        }
        None
    }

    /// Layer surface with exclusive keyboard grab (lock screens, launcher
    /// popups), if any. It takes precedence over tiled windows.
    fn exclusive_layer_focus(
        &self,
    ) -> Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface> {
        use smithay::wayland::shell::wlr_layer::KeyboardInteractivity;
        for output in self.space.outputs() {
            let map = smithay::desktop::layer_map_for_output(output);
            for layer in map.layers() {
                if layer.can_receive_keyboard_focus()
                    && layer.cached_state().keyboard_interactivity
                        == KeyboardInteractivity::Exclusive
                {
                    return Some(layer.wl_surface().clone());
                }
            }
        }
        None
    }

    /// Syncs Smithay keyboard focus + window activation with the workspace focus.
    pub fn update_keyboard_focus(&mut self) {
        let serial = SERIAL_COUNTER.next_serial();

        // Exclusive layer surfaces (lock screens) steal all keyboard input.
        if let Some(surface) = self.exclusive_layer_focus() {
            for window in self.space.elements() {
                window.set_activated(false);
            }
            let seat = self.seat.clone();
            if let Some(keyboard) = seat.get_keyboard() {
                keyboard.set_focus(self, Some(surface), serial);
            }
            return;
        }

        let focused_id = self.active_workspace().get_focused();
        let focused = self.focused_wl_surface();

        // Activate the focused window, deactivate the rest. Identity runs
        // through workspace ids so Wayland and X11 windows share the path.
        for window in self.space.elements() {
            let is_focused = focused_id
                .map(|fid| Some(fid) == self.id_of_window(window))
                .unwrap_or(false);
            window.set_activated(is_focused);
        }

        let seat = self.seat.clone();
        if let Some(keyboard) = seat.get_keyboard() {
            keyboard.set_focus(self, focused, serial);
        }

        if let Some(id) = self.active_workspace().get_focused() {
            self.broadcast_event(IpcEvent::FocusChanged(Some(id.as_usize())));
        }
    }

    /// Focuses a window by id (Wayland or X11). No-op for unknown ids.
    pub fn focus_window_id(&mut self, id: WindowId) {
        self.active_workspace_mut().focus_window(id);
        self.update_keyboard_focus();
    }

    pub fn focus_next(&mut self) {
        self.active_workspace_mut().focus_next();
        self.update_keyboard_focus();
    }

    pub fn focus_prev(&mut self) {
        self.active_workspace_mut().focus_prev();
        self.update_keyboard_focus();
    }

    /// Closes the currently focused window (`kill_active`).
    pub fn kill_active(&mut self) {
        if let Some(surface) = self.focused_surface() {
            surface.send_close();
        }
        #[cfg(feature = "xwayland")]
        if let Some(x11) = self.focused_x11_surface()
            && let Err(e) = x11.close()
        {
            tracing::warn!("failed to close X11 window: {e:?}");
        }
    }

    /// Activates a workspace, rebuilding the space for it. Out-of-range
    /// indices are ignored (IPC clients speak 1-based workspace numbers).
    pub fn switch_workspace(&mut self, idx: usize) {
        if idx >= self.workspaces.len() {
            tracing::warn!(
                "workspace switch to index {idx} ignored, only {} workspaces",
                self.workspaces.len()
            );
            return;
        }
        if idx == self.active_ws {
            return;
        }
        self.active_ws = idx;
        self.remap_active_workspace();
        self.broadcast_event(IpcEvent::WorkspaceSwitched(
            self.active_workspace().name.clone(),
        ));
    }

    pub fn next_workspace(&mut self) {
        let next = (self.active_ws + 1) % self.workspaces.len();
        self.switch_workspace(next);
    }

    pub fn prev_workspace(&mut self) {
        let prev = if self.active_ws == 0 {
            self.workspaces.len() - 1
        } else {
            self.active_ws - 1
        };
        self.switch_workspace(prev);
    }
}
