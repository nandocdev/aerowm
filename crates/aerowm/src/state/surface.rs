//! Backend-agnostic window lookup and configuration.
//!
//! Every managed window is addressable by [`WindowId`], whether it came
//! from Wayland or from XWayland. The helpers here — plus the
//! [`ConfigurableSurface`] trait — exist so that "tell this window its
//! new geometry" is written once instead of an `if Wayland / else X11`
//! branch repeated at every call site (tiling, floating remaps and
//! interactive resizes all need it).

use aerowm_core::geometry::Rect as CoreRect;
use aerowm_core::id::WindowId;
use smithay::desktop::Window;
use smithay::wayland::shell::xdg::ToplevelSurface;

#[cfg(feature = "xwayland")]
use smithay::utils::Rectangle;
#[cfg(feature = "xwayland")]
use smithay::xwayland::X11Surface;

use super::AerowmState;

/// A managed window that can be told a new geometry, independent of the
/// protocol it was mapped with.
pub trait ConfigurableSurface {
    /// Sends `rect` to the window. Implementations log protocol errors
    /// instead of propagating them: a client that refuses a configure
    /// must not take the compositor down.
    fn push_geometry(&self, rect: CoreRect);
}

impl ConfigurableSurface for ToplevelSurface {
    fn push_geometry(&self, rect: CoreRect) {
        // Wayland clients own their own position; only the size is
        // configurable (position comes from the compositor's space map).
        self.with_pending_state(|state| {
            state.size = Some((rect.size.width as i32, rect.size.height as i32).into());
        });
        self.send_configure();
    }
}

#[cfg(feature = "xwayland")]
impl ConfigurableSurface for X11Surface {
    fn push_geometry(&self, rect: CoreRect) {
        // X11 windows are told both position and size.
        let geo = Rectangle::new(
            (rect.origin.x, rect.origin.y).into(),
            (rect.size.width as i32, rect.size.height as i32).into(),
        );
        if let Err(e) = self.configure(Some(geo)) {
            tracing::warn!("failed to configure X11 window: {e:?}");
        }
    }
}

impl AerowmState {
    /// The configurable surface backing a window id, or `None` when the
    /// id is unknown. Wayland wins if an id somehow exists in both maps.
    pub(crate) fn configurable(&self, id: WindowId) -> Option<&dyn ConfigurableSurface> {
        if let Some(surface) = self.surfaces.get(&id) {
            return Some(surface);
        }
        #[cfg(feature = "xwayland")]
        if let Some(x11) = self.x11_surfaces.get(&id) {
            return Some(x11);
        }
        None
    }

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
