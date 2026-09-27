//! Session restore and config hot reload.
//!
//! Distinct from [`crate::session`], which serializes sessions to disk:
//! this module consumes the pending placements that module produces and
//! swaps in a reloaded Luau engine.

use aerowm_core::id::WindowId;
use smithay::desktop::Window;
use smithay::utils::Point;

use super::AerowmState;

impl AerowmState {
    /// Re-places a freshly mapped window according to restored session
    /// state. Consumes the first pending entry matching `app` and moves
    /// the window to its recorded workspace slot, floating geometry and
    /// focus. `prev_focus` is the active-workspace focus from before the
    /// map path focused the newcomer; it is restored unless the entry
    /// itself was focused. No-op when nothing matches.
    ///
    /// Callers map the window normally first; this fixes it up afterwards
    /// (still within the same dispatch, invisible to clients).
    pub fn apply_pending_placement(
        &mut self,
        id: WindowId,
        app: &aerowm_core::session::AppId,
        prev_focus: Option<WindowId>,
    ) {
        let entry = match crate::session::consume_placement(&mut self.pending_placements, app) {
            Some(entry) => entry,
            None => return,
        };
        let ws_idx = entry.workspace.min(self.workspaces.len().saturating_sub(1));
        // Detach from wherever the map path put it.
        for ws in &mut self.workspaces {
            ws.remove_window(id);
        }
        self.scratchpad.retain(|&x| x != id);
        if let Some(window) = self.window_object(id) {
            self.space.unmap_elem(&window);
        }
        self.float_geo.remove(&id);

        // Re-insert at the recorded stack position.
        let position = entry.position;
        let floating = entry.floating;
        let geo = entry.geo;
        let focused = entry.focused;
        {
            let ws = &mut self.workspaces[ws_idx];
            ws.insert_window(id, position);
            if floating {
                ws.set_floating(id, true);
            }
            if focused {
                ws.focus_window(id);
            }
        }
        if floating && let Some(geo) = geo {
            self.float_geo.insert(id, geo);
        }

        if ws_idx == self.active_ws {
            // Rebuild the space element for the active workspace.
            let window = self.window_object(id).or_else(|| {
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
            if let Some(window) = window {
                if floating {
                    let loc = self
                        .float_geo
                        .get(&id)
                        .map(|r| Point::from((r.origin.x, r.origin.y)))
                        .unwrap_or(Point::from((0, 0)));
                    self.push_pinned_geometry(id);
                    self.space.map_element(window, loc, false);
                } else {
                    self.space.map_element(window, (0, 0), false);
                }
            }
            // The map path focused the newcomer; restore recorded focus:
            // the flagged window, otherwise whoever had focus before it.
            if focused {
                self.workspaces[ws_idx].focus_window(id);
            } else if let Some(prev) = prev_focus {
                let ws = &mut self.workspaces[ws_idx];
                if ws.get_windows().contains(&prev) {
                    ws.focus_window(prev);
                }
            }
            self.apply_layout();
            self.update_keyboard_focus();
        }
    }

    /// Performs an atomic Hot Reload of the Luau configuration.
    pub fn reload_config(&mut self) -> Result<(), String> {
        let new_engine = aerowm_lua::ScriptEngine::new()
            .map_err(|e| format!("Failed to init new Lua engine: {}", e))?;
        new_engine
            .load_default_config()
            .map_err(|e| format!("Config syntax error: {}", e))?;

        self.engine = new_engine;
        tracing::info!("Hot reload successful");
        let _ = self.engine.emit_hook("reload");
        Ok(())
    }
}
