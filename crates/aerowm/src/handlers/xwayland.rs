//! XWayland integration: XWM event handling and xwayland-shell protocol.
//!
//! Managed X11 windows join workspaces, the tiling layout, focus, grabs
//! and IPC exactly like Wayland ones (keyed by the same [`WindowId`]).
//! Override-redirect windows (menus, tooltips) are mapped unmanaged at
//! client geometry: visible and pointer-interactive, but outside focus
//! and layouts.

use smithay::desktop::Window;
use smithay::utils::{Logical, Rectangle};
use smithay::wayland::xwayland_shell::{XWaylandShellHandler, XWaylandShellState};
use smithay::xwayland::{
    X11Surface, X11Wm, XwmHandler,
    xwm::{Reorder, ResizeEdge, WmWindowProperty, X11Window, XwmId},
};
use smithay::delegate_xwayland_shell;

use aerowm_core::geometry::Rect as CoreRect;
use aerowm_core::id::WindowId;

use crate::state::{AerowmState, MIN_FLOAT_H, MIN_FLOAT_W};

impl XWaylandShellHandler for AerowmState {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell
    }
}

/// Looks up the workspace id for a managed X11 surface.
fn id_for_x11(state: &AerowmState, surface: &X11Surface) -> Option<WindowId> {
    let wid = surface.window_id();
    state
        .x11_surfaces
        .iter()
        .find(|(_, s)| s.window_id() == wid)
        .map(|(id, _)| *id)
}

/// Finds the mapped [`Window`] backing an X11 window id, if any.
fn window_for_xid(state: &AerowmState, wid: X11Window) -> Option<Window> {
    state
        .space
        .elements()
        .find(|w| w.x11_surface().map(|s| s.window_id()) == Some(wid))
        .cloned()
}

impl XwmHandler for AerowmState {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwm
            .as_mut()
            .expect("XWM event with no window manager running")
    }

    fn new_window(&mut self, _xwm: XwmId, window: X11Surface) {
        tracing::debug!("X11 window created: {:?}", window.window_id());
    }

    fn new_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        tracing::debug!("X11 override-redirect window: {:?}", window.window_id());
    }

    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        let prev_focus = self.active_workspace().get_focused();
        let app = crate::session::app_id_of_x11(&window);
        match id_for_x11(self, &window) {
            Some(id) => {
                // Re-map of a known surface (e.g. after unmap): restore
                // workspace membership without duplicating tracking.
                if !self.active_workspace().get_windows().contains(&id) {
                    self.active_workspace_mut().add_window(id);
                    self.broadcast_event(aerowm_ipc::IpcEvent::WindowOpened(id.as_usize()));
                }
            }
            None => {
                let id = WindowId::new();
                self.active_workspace_mut().add_window(id);
                self.x11_surfaces.insert(id, window.clone());
                self.broadcast_event(aerowm_ipc::IpcEvent::WindowOpened(id.as_usize()));
            }
        }

        if window_for_xid(self, window.window_id()).is_none() {
            let w = Window::new_x11_window(window.clone());
            self.space.map_element(w, (0, 0), true);
        }
        // Restored session placement (no-op unless a pending entry matches).
        if let Some(app) = app {
            if let Some(id) = id_for_x11(self, &window) {
                self.apply_pending_placement(id, &app, prev_focus);
            }
        }
        self.apply_layout();
        self.update_keyboard_focus();

        if let Err(e) = window.set_mapped(true) {
            tracing::warn!("granting X11 map request failed: {e:?}");
        }
    }

    fn map_window_notify(&mut self, _xwm: XwmId, _window: X11Surface) {
        // Surface is live now: (re-)tile with real geometry and focus it.
        self.apply_layout();
        self.update_keyboard_focus();
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        let wid = window.window_id();
        if !self.override_redirect.iter().any(|s| s.window_id() == wid) {
            self.override_redirect.push(window.clone());
        }
        // Client geometry is authoritative for unmanaged windows.
        let geo = window.geometry();
        let w = Window::new_x11_window(window);
        self.space.map_element(w, geo.loc, true);
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.remove_x11_window(&window);
    }

    fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.remove_x11_window(&window);
    }

    #[allow(clippy::too_many_arguments)]
    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        let Some(id) = id_for_x11(self, &window) else {
            return;
        };

        if self.active_workspace().is_floating(id) {
            // Floating: honor the client request merged over pinned geometry.
            let base = self
                .float_geo
                .get(&id)
                .copied()
                .unwrap_or(CoreRect::new(0, 0, 800, 600));
            let nw = w.map(|w| w.max(1) as i32).unwrap_or(base.size.width as i32)
                .max(MIN_FLOAT_W);
            let nh = h.map(|h| h.max(1) as i32).unwrap_or(base.size.height as i32)
                .max(MIN_FLOAT_H);
            let nx = x.unwrap_or(base.origin.x);
            let ny = y.unwrap_or(base.origin.y);
            let geo = Rectangle::<i32, Logical>::new((nx, ny).into(), (nw, nh).into());
            if let Err(e) = window.configure(Some(geo)) {
                tracing::warn!("X11 configure failed: {e:?}");
            }
            if let Some(w) = window_for_xid(self, window.window_id()) {
                self.space.map_element(w, geo.loc, false);
            }
            self.float_geo.insert(
                id,
                CoreRect::new(nx, ny, nw as u32, nh as u32),
            );
        } else {
            // Tiled: the layout is authoritative; re-assert it.
            self.apply_layout();
        }
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        geometry: Rectangle<i32, Logical>,
        _above: Option<X11Window>,
    ) {
        let wid = window.window_id();
        if let Some(id) = id_for_x11(self, &window) {
            // Keep pinned geometry in sync for floating windows.
            if self.active_workspace().is_floating(id) {
                self.float_geo.insert(
                    id,
                    CoreRect::new(
                        geometry.loc.x,
                        geometry.loc.y,
                        geometry.size.w.max(0) as u32,
                        geometry.size.h.max(0) as u32,
                    ),
                );
                if let Some(w) = window_for_xid(self, wid) {
                    self.space.map_element(w, geometry.loc, false);
                }
            }
        } else if self
            .override_redirect
            .iter()
            .any(|s| s.window_id() == wid)
        {
            // Unmanaged windows follow client geometry directly.
            if let Some(w) = window_for_xid(self, wid) {
                self.space.map_element(w, geometry.loc, false);
            }
        }
    }

    fn maximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        // Tiling WMs honor maximize by floating over the usable area.
        let Some(id) = id_for_x11(self, &window) else {
            return;
        };
        self.active_workspace_mut().set_floating(id, true);
        let area = self
            .output
            .clone()
            .map(|o| self.usable_area_for_output(&o))
            .unwrap_or(CoreRect::new(0, 0, 1920, 1080));
        self.float_geo.insert(id, area);
        self.apply_layout();
        let geo = Rectangle::<i32, Logical>::new(
            (area.origin.x, area.origin.y).into(),
            (area.size.width as i32, area.size.height as i32).into(),
        );
        if let Err(e) = window.configure(Some(geo)) {
            tracing::warn!("X11 maximize configure failed: {e:?}");
        }
        if let Err(e) = window.set_maximized(true) {
            tracing::warn!("X11 set_maximized failed: {e:?}");
        }
    }

    fn unmaximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        let Some(id) = id_for_x11(self, &window) else {
            return;
        };
        self.active_workspace_mut().set_floating(id, false);
        self.float_geo.remove(&id);
        if let Err(e) = window.set_maximized(false) {
            tracing::warn!("X11 set_maximized(false) failed: {e:?}");
        }
        self.apply_layout();
    }

    fn property_notify(&mut self, _xwm: XwmId, _window: X11Surface, _property: WmWindowProperty) {
        // Title/class changes don't affect tiling; ignored for now.
    }

    /// Client-initiated interactive move (`_NET_WM_MOVERESIZE`): drive it
    /// with the same grab machinery as `Super`-drag.
    fn move_request(&mut self, _xwm: XwmId, window: X11Surface, button: u32) {
        if let Some(id) = id_for_x11(self, &window) {
            let cursor = self.pointer_location;
            self.begin_move_grab(id, button, cursor);
        }
    }

    /// Client-initiated interactive resize. Our engine resizes from
    /// corners; the cursor already sits on the requested edge, so the
    /// nearest-corner pick inside [`AerowmState::begin_resize_grab`] agrees.
    fn resize_request(&mut self, _xwm: XwmId, window: X11Surface, button: u32, _edge: ResizeEdge) {
        if let Some(id) = id_for_x11(self, &window) {
            let cursor = self.pointer_location;
            self.begin_resize_grab(id, button, cursor);
        }
    }
}

delegate_xwayland_shell!(AerowmState);
