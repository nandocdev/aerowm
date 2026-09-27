//! Declarative window rules from the Luau config.

use aerowm_core::id::WindowId;
use smithay::wayland::shell::xdg::ToplevelSurface;

use super::AerowmState;

impl AerowmState {
    /// Applies declarative Window Rules via Luau to a new window.
    pub fn apply_window_rules(&mut self, id: WindowId, app_id: &str, title: Option<&str>) {
        let rules = self.engine.evaluate_rules(app_id, title);

        if let Some(ws_idx) = rules.workspace {
            let target_ws = ws_idx.saturating_sub(1);
            if target_ws >= self.workspaces.len() {
                tracing::warn!(
                    "window rule targets workspace {ws_idx}, only {} exist; ignoring",
                    self.workspaces.len()
                );
            } else if target_ws != self.active_ws {
                // Remove from whichever workspace currently holds it:
                // rules are re-evaluated on app_id/title change, so this
                // must be idempotent (never duplicate the entry).
                for ws in &mut self.workspaces {
                    ws.remove_window(id);
                }
                self.workspaces[target_ws].add_window(id);
            }
        }

        if let Some(scratchpad) = rules.scratchpad
            && scratchpad
            && !self.scratchpad.contains(&id)
        {
            self.scratchpad.push(id);
            for ws in &mut self.workspaces {
                ws.remove_window(id);
            }
            if let Some(window) = self.window_object(id) {
                self.space.unmap_elem(&window);
            }
            self.float_window(id);
        }
        if let Some(floating) = rules.floating
            && floating
        {
            self.float_window(id);
        }
    }

    /// Re-evaluates Luau window rules for a mapped toplevel whose identity
    /// changed (see `app_id_changed`/`title_changed`: at `new_toplevel`
    /// time app_id/title are still empty). Idempotent.
    pub fn reapply_rules_for_toplevel(&mut self, surface: &ToplevelSurface) {
        let Some((&id, _)) = self.surfaces.iter().find(|(_, s)| *s == surface) else {
            return;
        };
        let app = crate::session::app_id_of_toplevel(surface);
        let app_id = app.as_ref().map(|a| a.id.as_str()).unwrap_or("");
        let title = app.as_ref().and_then(|a| a.title.as_deref());
        self.apply_window_rules(id, app_id, title);
        self.apply_layout();
        self.update_keyboard_focus();
    }
}
