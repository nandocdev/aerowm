use std::collections::HashSet;

use crate::id::WindowId;
use crate::geometry::Rect;
use crate::layout::{Layout, LayoutSpec};

/// Container for managing windows and focus in a virtual workspace.
///
/// Windows marked as *floating* keep a user-controlled geometry (dragged
/// and resized with the pointer) and are excluded from the tiling layout.
pub struct Workspace {
    pub name: String,
    windows: Vec<WindowId>,
    focused_index: Option<usize>,
    layout: Box<dyn Layout>,
    /// Serializable description of `layout` (name + parameters). Kept in
    /// sync by [`Workspace::set_layout`]; any future runtime tuning of
    /// layout parameters must update it too so sessions round-trip.
    spec: LayoutSpec,
    floating: HashSet<WindowId>,
}

impl Workspace {
    pub fn new(name: impl Into<String>) -> Self {
        let spec = LayoutSpec::default();
        Self {
            name: name.into(),
            windows: Vec::new(),
            focused_index: None,
            layout: spec.instantiate(),
            spec,
            floating: HashSet::new(),
        }
    }

    /// Returns a reference to the current layout.
    pub fn get_current_layout(&self) -> &dyn Layout {
        self.layout.as_ref()
    }

    /// Serializable description of the current layout (name + parameters).
    pub fn layout_spec(&self) -> LayoutSpec {
        self.spec.clone()
    }

    /// Replaces the layout algorithm from a serializable spec.
    pub fn set_layout(&mut self, spec: LayoutSpec) {
        self.layout = spec.instantiate();
        self.spec = spec;
    }

    /// Adds a window to the workspace and focuses it.
    pub fn add_window(&mut self, id: WindowId) {
        self.windows.push(id);
        self.focused_index = Some(self.windows.len() - 1);
    }

    /// Inserts a window at a stack position without touching focus.
    /// Used by session restore to rebuild recorded stacking order.
    /// Unknown positions clamp into range; duplicates are ignored.
    pub fn insert_window(&mut self, id: WindowId, position: usize) {
        if self.windows.contains(&id) {
            return;
        }
        let pos = position.min(self.windows.len());
        self.windows.insert(pos, id);
        if let Some(focused) = self.focused_index {
            // Keep focus pointing at the same window after the shift.
            if pos <= focused {
                self.focused_index = Some(focused + 1);
            }
        }
    }

    /// Removes a window, gracefully adjusting the focus.
    pub fn remove_window(&mut self, id: WindowId) {
        self.floating.remove(&id);
        if let Some(pos) = self.windows.iter().position(|&w| w == id) {
            self.windows.remove(pos);
            if self.windows.is_empty() {
                self.focused_index = None;
            } else if let Some(focused) = self.focused_index {
                // If the removed window was before the focused one, shift focus index left
                if pos < focused || focused >= self.windows.len() {
                    self.focused_index = Some(focused.saturating_sub(1));
                }
            }
        }
    }

    pub fn get_focused(&self) -> Option<WindowId> {
        self.focused_index.map(|idx| self.windows[idx])
    }

    /// Directly focuses the given window if it belongs to this workspace.
    pub fn focus_window(&mut self, id: WindowId) {
        if let Some(pos) = self.windows.iter().position(|&w| w == id) {
            self.focused_index = Some(pos);
        }
    }

    /// Shifts focus to the next window in the stack.
    pub fn focus_next(&mut self) {
        if self.windows.is_empty() { return; }
        if let Some(idx) = self.focused_index {
            self.focused_index = Some((idx + 1) % self.windows.len());
        }
    }

    /// Shifts focus to the previous window in the stack.
    pub fn focus_prev(&mut self) {
        if self.windows.is_empty() { return; }
        if let Some(idx) = self.focused_index {
            self.focused_index = Some(if idx == 0 {
                self.windows.len() - 1
            } else {
                idx - 1
            });
        }
    }

    /// Swaps the currently focused window with the master (first) window.
    pub fn swap_master(&mut self) {
        if self.windows.len() < 2 { return; }
        if let Some(idx) = self.focused_index {
            if idx != 0 {
                self.windows.swap(0, idx);
                self.focused_index = Some(0);
            }
        }
    }

    pub fn get_windows(&self) -> &[WindowId] {
        &self.windows
    }

    /// Marks a window as floating (user geometry) or tiled. No-op for unknown ids.
    /// Returns the new floating state (`false` for unknown ids).
    pub fn set_floating(&mut self, id: WindowId, floating: bool) -> bool {
        if !self.windows.contains(&id) {
            return false;
        }
        if floating {
            self.floating.insert(id);
        } else {
            self.floating.remove(&id);
        }
        floating
    }

    /// Toggles the floating state. Returns the new state (`false` for unknown ids).
    pub fn toggle_floating(&mut self, id: WindowId) -> bool {
        if !self.windows.contains(&id) {
            return false;
        }
        if self.floating.contains(&id) {
            self.floating.remove(&id);
            false
        } else {
            self.floating.insert(id);
            true
        }
    }

    pub fn is_floating(&self, id: WindowId) -> bool {
        self.floating.contains(&id)
    }

    /// Applies the layout algorithm mapping each *tiled* WindowId to its
    /// calculated Rect. Floating windows are excluded — the compositor keeps
    /// their user-controlled geometry.
    pub fn apply_layout(&self, layout: &dyn Layout, area: Rect) -> Vec<(WindowId, Rect)> {
        let tiled: Vec<WindowId> = self
            .windows
            .iter()
            .copied()
            .filter(|id| !self.floating.contains(id))
            .collect();
        let rects = layout.apply(area, tiled.len());
        tiled.into_iter().zip(rects).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layouts::MonadTall;

    #[test]
    fn test_add_remove_and_focus() {
        let mut ws = Workspace::new("1");
        let w1 = WindowId::new();
        let w2 = WindowId::new();

        assert_eq!(ws.get_focused(), None);
        
        ws.add_window(w1);
        assert_eq!(ws.get_focused(), Some(w1));
        
        ws.add_window(w2);
        assert_eq!(ws.get_focused(), Some(w2));
        
        ws.remove_window(w1);
        assert_eq!(ws.get_focused(), Some(w2));
        
        ws.remove_window(w2);
        assert_eq!(ws.get_focused(), None);
    }

    #[test]
    fn test_focus_navigation() {
        let mut ws = Workspace::new("1");
        let w1 = WindowId::new();
        let w2 = WindowId::new();
        let w3 = WindowId::new();

        ws.add_window(w1);
        ws.add_window(w2);
        ws.add_window(w3);
        
        // newly added w3 is focused
        assert_eq!(ws.get_focused(), Some(w3));
        
        ws.focus_next(); // wrap around to w1
        assert_eq!(ws.get_focused(), Some(w1));
        
        ws.focus_prev(); // wrap back to w3
        assert_eq!(ws.get_focused(), Some(w3));
    }

    #[test]
    fn test_floating_excluded_from_layout() {
        let mut ws = Workspace::new("1");
        let w1 = WindowId::new();
        let w2 = WindowId::new();

        ws.add_window(w1);
        ws.add_window(w2);
        assert!(!ws.is_floating(w1));

        assert!(ws.toggle_floating(w1));
        assert!(ws.is_floating(w1));

        let area = Rect::new(0, 0, 1920, 1080);
        let layout = MonadTall::default();
        let placed = ws.apply_layout(&layout, area);
        // Only the tiled window is placed, floating keeps user geometry.
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].0, w2);

        // Back to tiled: both placed again.
        assert!(!ws.toggle_floating(w1));
        let placed = ws.apply_layout(&layout, area);
        assert_eq!(placed.len(), 2);

        // Unknown ids are inert.
        let ghost = WindowId::new();
        assert!(!ws.toggle_floating(ghost));
        assert!(!ws.set_floating(ghost, true));
        assert!(!ws.is_floating(ghost));

        // Removal clears floating state.
        ws.set_floating(w1, true);
        ws.remove_window(w1);
        assert!(!ws.is_floating(w1));
    }

    #[test]
    fn test_swap_master() {
        let mut ws = Workspace::new("1");
        let w1 = WindowId::new();
        let w2 = WindowId::new();
        let w3 = WindowId::new();

        ws.add_window(w1); // master
        ws.add_window(w2);
        ws.add_window(w3); // currently focused
        
        ws.swap_master();
        
        let windows = ws.get_windows();
        assert_eq!(windows[0], w3); // w3 is now master
        assert_eq!(windows[2], w1); // w1 moved to the end
        assert_eq!(ws.get_focused(), Some(w3)); // w3 remains focused
    }
}
