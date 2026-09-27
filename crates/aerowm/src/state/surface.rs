//! Backend-agnostic window lookup: every managed window is addressable
//! by [`WindowId`], whether it came from Wayland or from XWayland.

use aerowm_core::id::WindowId;
use smithay::desktop::Window;

use super::AerowmState;

impl AerowmState {
    /// Window object currently mapped for a window id, if any.
    pub(crate) fn window_object(&self, id: WindowId) -> Option<Window> {
        if let Some(surface) = self.surfaces.get(&id)
            && let Some(window) = self
                .space
                .elements()
                .find(|w| w.toplevel() == Some(surface))
        {
            return Some(window.clone());
        }
        #[cfg(feature = "xwayland")]
        if let Some(x11) = self.x11_surfaces.get(&id) {
            let wid = x11.window_id();
            if let Some(window) = self
                .space
                .elements()
                .find(|w| w.x11_surface().map(|s| s.window_id()) == Some(wid))
            {
                return Some(window.clone());
            }
        }
        None
    }

    /// Resolves the workspace id backing a mapped `Window`, if any.
    /// Override-redirect windows are unmanaged and return `None`.
    pub(crate) fn id_of_window(&self, window: &Window) -> Option<WindowId> {
        if let Some(tl) = window.toplevel() {
            return self
                .surfaces
                .iter()
                .find(|(_, s)| *s == tl)
                .map(|(id, _)| *id);
        }
        #[cfg(feature = "xwayland")]
        if let Some(x11) = window.x11_surface() {
            let wid = x11.window_id();
            return self
                .x11_surfaces
                .iter()
                .find(|(_, s)| s.window_id() == wid)
                .map(|(id, _)| *id);
        }
        None
    }
}
