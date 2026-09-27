//! XWayland lifecycle: server startup, X11 window manager hand-off, and
//! full removal of managed X11 windows.
//!
//! Everything here is behind the `xwayland` feature; the shared window
//! paths (focus, layout, grabs, rules) treat X11 windows as first-class
//! through [`super::surface::ConfigurableSurface`] and window ids.

use calloop::LoopHandle;
use smithay::desktop::Window;
use smithay::xwayland::{X11Surface, X11Wm, XWayland, XWaylandEvent};
use wayland_server::DisplayHandle;

use super::{AerowmState, PointerGrabState};

impl AerowmState {
    /// Starts the XWayland server and, once ready, the X11 window manager.
    ///
    /// Missing `Xwayland` binary (or any other startup failure) is reported
    /// as `Err` so the caller can keep running Wayland-only.
    pub fn init_xwayland(
        &mut self,
        display_handle: &DisplayHandle,
        loop_handle: LoopHandle<'static, Self>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.loop_handle = Some(loop_handle.clone());

        let (xwayland, client) = XWayland::spawn(
            display_handle,
            None,
            Vec::<(String, String)>::new(),
            true,
            std::process::Stdio::null(),
            std::process::Stdio::null(),
            |_| {},
        )
        .map_err(|e| format!("spawning Xwayland failed (is it installed?): {e}"))?;

        loop_handle
            .insert_source(xwayland, move |event, _, state: &mut Self| {
                match event {
                    XWaylandEvent::Ready {
                        x11_socket,
                        display_number,
                    } => {
                        tracing::info!("XWayland ready on :{display_number}");
                        // SAFETY: single-threaded startup path, no concurrent
                        // readers of the process environment here.
                        unsafe {
                            std::env::set_var("DISPLAY", format!(":{display_number}"));
                        }
                        let Some(handle) = state.loop_handle.clone() else {
                            tracing::error!("no loop handle for XWM startup");
                            return;
                        };
                        match X11Wm::start_wm(handle, x11_socket, client.clone()) {
                            Ok(xwm) => {
                                tracing::info!("X11 window manager started");
                                state.xwm = Some(xwm);
                            }
                            Err(e) => {
                                tracing::error!("starting XWM failed: {e:?}");
                            }
                        }
                    }
                    XWaylandEvent::Error => {
                        tracing::error!("XWayland exited during startup; X11 apps unavailable");
                    }
                }
            })
            .map_err(|e| format!("XWayland event source: {e}"))?;

        Ok(())
    }

    /// Fully removes a managed X11 window: workspace, maps, geometry,
    /// grabs and space. Idempotent — safe to call from both unmap and
    /// destroy notifications.
    pub fn remove_x11_window(&mut self, surface: &X11Surface) {
        let wid = surface.window_id();
        let id = self
            .x11_surfaces
            .iter()
            .find(|(_, s)| s.window_id() == wid)
            .map(|(id, _)| *id);
        if let Some(id) = id {
            self.x11_surfaces.remove(&id);
            self.float_geo.remove(&id);
            if self.grab.window_id() == Some(id) {
                self.grab = PointerGrabState::None;
            }
            for ws in &mut self.workspaces {
                ws.remove_window(id);
            }
            self.broadcast_event(aerowm_ipc::IpcEvent::WindowClosed(id.as_usize()));
        }
        // Unmap any matching space element (managed or override-redirect).
        let dead: Vec<Window> = self
            .space
            .elements()
            .filter(|w| {
                w.x11_surface()
                    .map(|s| s.window_id() == wid)
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
        for w in dead {
            self.space.unmap_elem(&w);
        }
        self.override_redirect.retain(|s| s.window_id() != wid);
        self.apply_layout();
        self.update_keyboard_focus();
    }
}
