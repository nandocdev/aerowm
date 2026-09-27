//! Tiling, space mapping and floating geometry.
//!
//! Geometry reaches clients through [`ConfigurableSurface`], so a new
//! backend only has to implement that one trait instead of being
//! threaded through the layout code.

use aerowm_core::geometry::Rect as CoreRect;
use aerowm_core::id::WindowId;
use smithay::desktop::Window;
use smithay::output::Output;
use smithay::utils::Point;

use super::AerowmState;
use std::collections::HashMap;

impl AerowmState {
    /// Usable tiling area of an output: full output geometry minus the
    /// exclusive zones reserved by layer-shell surfaces (bars, panels).
    pub fn usable_area_for_output(&self, output: &Output) -> CoreRect {
        let zone = smithay::desktop::layer_map_for_output(output).non_exclusive_zone();
        let origin = self
            .space
            .output_geometry(output)
            .map(|g| g.loc)
            .unwrap_or_default();
        CoreRect::new(
            origin.x + zone.loc.x,
            origin.y + zone.loc.y,
            zone.size.w.max(0) as u32,
            zone.size.h.max(0) as u32,
        )
    }

    /// Apply the current layout to all windows in the active workspace
    pub fn apply_layout(&mut self) {
        // Tile inside the usable area so layer-shell bars/panels are
        // never covered by tiled windows.
        let area = match self.output.clone() {
            Some(output) => self.usable_area_for_output(&output),
            None => {
                let Some(output_size) = self.output_size else {
                    return;
                };
                CoreRect::new(0, 0, output_size.w as u32, output_size.h as u32)
            }
        };
        self.apply_layout_in(area);
    }

    /// Same as [`Self::apply_layout`] but for an explicit area.
    /// Used by the native backend to tile per-monitor.
    pub fn apply_layout_in(&mut self, area: CoreRect) {
        let layout = self.active_workspace().get_current_layout();
        let window_rects = self.active_workspace().apply_layout(layout, area);

        // Collect windows to move first to avoid borrow issues
        let mut windows_to_move = Vec::new();
        for (window_id, rect) in window_rects {
            if let Some(surface) = self.configurable(window_id) {
                surface.push_geometry(rect);
            }
            if let Some(window) = self.window_object(window_id) {
                windows_to_move.push((window, Point::from((rect.origin.x, rect.origin.y))));
            }
        }

        for (window, location) in windows_to_move {
            self.space.map_element(window, location, true);
        }
    }

    /// Re-sends a floating window's pinned geometry to its client. Used
    /// after a space remap, where the client would otherwise be left with
    /// whatever size the previous layout left it at.
    pub(crate) fn push_pinned_geometry(&self, id: WindowId) {
        if let Some(pinned) = self.float_geo.get(&id).copied()
            && let Some(surface) = self.configurable(id)
        {
            surface.push_geometry(pinned);
        }
    }

    /// Re-creates every space element of the active workspace, then
    /// re-tiles. Runs after a workspace switch.
    pub(crate) fn remap_active_workspace(&mut self) {
        // Remember Window objects by id before unmapping everything.
        // Managed X11 windows ride along via the same id lookup.
        let mut by_id: HashMap<WindowId, Window> = HashMap::new();
        for window in self.space.elements() {
            if let Some(id) = self.id_of_window(window) {
                by_id.insert(id, window.clone());
            }
        }
        let all: Vec<Window> = self.space.elements().cloned().collect();
        for w in all {
            self.space.unmap_elem(&w);
        }
        let ids: Vec<WindowId> = self.active_workspace().get_windows().to_vec();
        for id in ids {
            // Resolve (or re-create) the Window from either backend.
            let window = by_id.remove(&id).or_else(|| {
                self.surfaces
                    .get(&id)
                    .cloned()
                    .map(Window::new_wayland_window)
            });
            #[cfg(feature = "xwayland")]
            let window = window.or_else(|| {
                self.x11_surfaces
                    .get(&id)
                    .cloned()
                    .map(Window::new_x11_window)
            });
            let Some(window) = window else { continue };
            if self.active_workspace().is_floating(id) {
                // Floating windows keep their pinned user geometry.
                let loc = self
                    .float_geo
                    .get(&id)
                    .map(|r| Point::from((r.origin.x, r.origin.y)))
                    .unwrap_or(Point::from((0, 0)));
                self.push_pinned_geometry(id);
                self.space.map_element(window, loc, false);
            } else {
                // Re-map; real position comes from apply_layout right after.
                self.space.map_element(window, (0, 0), false);
            }
        }
        self.apply_layout();
        self.update_keyboard_focus();
    }

    /// Promotes the master window, re-tiling around the new order.
    pub fn swap_master(&mut self) {
        self.active_workspace_mut().swap_master();
        self.apply_layout();
    }
}
