use crate::id::WindowId;
use crate::geometry::Rect;
use crate::layout::Layout;

/// Container for managing windows and focus in a virtual workspace.
#[derive(Debug)]
pub struct Workspace {
    pub name: String,
    windows: Vec<WindowId>,
    focused_index: Option<usize>,
}

impl Workspace {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            windows: Vec::new(),
            focused_index: None,
        }
    }

    /// Adds a window to the workspace and focuses it.
    pub fn add_window(&mut self, id: WindowId) {
        self.windows.push(id);
        self.focused_index = Some(self.windows.len() - 1);
    }

    /// Removes a window, gracefully adjusting the focus.
    pub fn remove_window(&mut self, id: WindowId) {
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

    /// Applies the layout algorithm mapping each WindowId to its calculated Rect.
    pub fn apply_layout(&self, layout: &dyn Layout, area: Rect) -> Vec<(WindowId, Rect)> {
        let rects = layout.apply(area, self.windows.len());
        self.windows.iter().copied().zip(rects.into_iter()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
