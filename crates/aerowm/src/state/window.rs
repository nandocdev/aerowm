//! Per-window bookkeeping: floating state, the scratchpad, and cleanup
//! of windows the client has destroyed.

use aerowm_core::geometry::Rect as CoreRect;
use aerowm_core::id::WindowId;
use smithay::utils::IsAlive;

use super::{AerowmState, PointerGrabState};

impl AerowmState {
    /// Current space geometry (location + committed size) of a window id.
    fn window_rect(&self, id: WindowId) -> Option<CoreRect> {
        let window = self.window_object(id)?;
        let loc = self.space.element_location(&window)?;
        let size = window.geometry().size;
        Some(CoreRect::new(
            loc.x,
            loc.y,
            size.w.max(0) as u32,
            size.h.max(0) as u32,
        ))
    }

    /// Marks a tiled window as floating, pinning its current geometry.
    /// Returns `true` if the window newly became floating.
    pub(crate) fn float_window(&mut self, id: WindowId) -> bool {
        if !self.surfaces.contains_key(&id) || self.active_workspace().is_floating(id) {
            return false;
        }
        self.active_workspace_mut().set_floating(id, true);
        if let Some(rect) = self.window_rect(id) {
            self.float_geo.insert(id, rect);
        }
        true
    }

    /// Toggles floating state of the focused window.
    pub fn toggle_floating(&mut self) {
        let Some(id) = self.active_workspace().get_focused() else {
            return;
        };
        if !self.surfaces.contains_key(&id) {
            return;
        }
        if self.active_workspace_mut().toggle_floating(id) {
            if let Some(rect) = self.window_rect(id) {
                self.float_geo.insert(id, rect);
            }
        } else {
            self.float_geo.remove(&id);
        }
        self.apply_layout();
    }

    /// Shows the most recent scratchpad window, or hides the focused one
    /// if it is already showing. At most one scratchpad window is visible.
    pub fn toggle_scratchpad(&mut self) {
        if self.scratchpad.is_empty() {
            return;
        }

        let mut focused_sp_idx = None;
        if let Some(focused) = self.active_workspace().get_focused()
            && let Some(pos) = self.scratchpad.iter().position(|&w| w == focused)
        {
            focused_sp_idx = Some(pos);
        }

        if let Some(pos) = focused_sp_idx {
            let id = self.scratchpad.remove(pos);
            self.scratchpad.push(id);
            self.active_workspace_mut().remove_window(id);
            if let Some(window) = self.window_object(id) {
                self.space.unmap_elem(&window);
            }
        } else {
            let id = self.scratchpad.last().copied().unwrap();
            let active_ws_idx = self.active_ws;
            for ws in &mut self.workspaces {
                ws.remove_window(id);
            }
            self.workspaces[active_ws_idx].add_window(id);
            self.float_window(id);
            self.workspaces[active_ws_idx].focus_window(id);
            self.update_keyboard_focus();
        }
        self.apply_layout();
    }

    /// Clean up dead windows (closed by client)
    pub fn cleanup_dead_windows(&mut self) {
        let dead_window_ids: Vec<_> = self
            .space
            .elements()
            .filter(|w| !w.alive())
            .filter_map(|w| self.id_of_window(w))
            .collect();

        for id in dead_window_ids {
            self.surfaces.remove(&id);
            self.float_geo.remove(&id);
            if self.grab.window_id() == Some(id) {
                self.grab = PointerGrabState::None;
            }
            for ws in &mut self.workspaces {
                ws.remove_window(id);
            }
            self.broadcast_event(aerowm_ipc::IpcEvent::WindowClosed(id.as_usize()));
        }

        // Unmap dead windows from space - collect first to avoid borrow issues
        let dead_windows: Vec<_> = self
            .space
            .elements()
            .filter(|w| !w.alive())
            .cloned()
            .collect();
        for w in dead_windows {
            self.space.unmap_elem(&w);
        }

        // Prune tracked override-redirect windows that died without an
        // explicit destroy notification.
        #[cfg(feature = "xwayland")]
        self.override_redirect.retain(|s| s.alive());
    }
}
