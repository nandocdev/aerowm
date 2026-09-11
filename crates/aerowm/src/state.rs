use aerowm_lua::ScriptEngine;
use wayland_server::DisplayHandle;
use std::os::unix::net::UnixStream;
use std::io::Write;
use aerowm_ipc::IpcEvent;

use smithay::wayland::compositor::CompositorState;
use smithay::wayland::shm::ShmState;
use smithay::wayland::shell::wlr_layer::WlrLayerShellState;
use smithay::wayland::shell::xdg::{XdgShellState, ToplevelSurface};
use smithay::wayland::fractional_scale::FractionalScaleManagerState;
use smithay::wayland::viewporter::ViewporterState;
use smithay::wayland::shell::xdg::decoration::XdgDecorationState;

#[cfg(feature = "xwayland")]
use smithay::wayland::xwayland_shell::XWaylandShellState;
#[cfg(feature = "xwayland")]
use smithay::xwayland::{X11Surface, X11Wm, XWayland, XWaylandEvent};
#[cfg(feature = "xwayland")]
use calloop::LoopHandle;
use smithay::input::{Seat, SeatState, keyboard::XkbConfig};
use smithay::desktop::space::Space;
use smithay::desktop::Window;
use smithay::desktop::PopupManager;
use smithay::output::Output;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::WinitGraphicsBackend;
use crate::backend::udev::UdevRuntime;
use smithay::utils::{Logical, Point, Size, IsAlive, SERIAL_COUNTER};
use smithay::wayland::output::{OutputHandler, OutputManagerState};

use std::collections::HashMap;
use aerowm_core::id::WindowId;
use aerowm_core::session::PendingPlacement;
use aerowm_core::workspace::Workspace;
use aerowm_core::geometry::Rect as CoreRect;
use crate::wayland_socket::WaylandSocketInfo;

pub const NUM_WORKSPACES: usize = 4;

/// Minimum user-resizable size for floating windows.
pub const MIN_FLOAT_W: i32 = 120;
pub const MIN_FLOAT_H: i32 = 80;

/// Corner grabbed for an interactive resize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrabCorner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Pointer grab state for interactive move/resize of floating windows.
///
/// While a grab is active, pointer motion and the grabbing button are
/// consumed by the compositor (not forwarded to clients).
#[derive(Debug, Clone, Copy)]
pub enum PointerGrabState {
    None,
    Move {
        id: WindowId,
        button: u32,
        dx: f64,
        dy: f64,
    },
    Resize {
        id: WindowId,
        button: u32,
        corner: GrabCorner,
        start: CoreRect,
        cursor_x: f64,
        cursor_y: f64,
    },
}

impl PointerGrabState {
    pub fn is_active(&self) -> bool {
        !matches!(self, PointerGrabState::None)
    }

    /// Button that initiated the grab, if any.
    pub fn button(&self) -> Option<u32> {
        match *self {
            PointerGrabState::None => None,
            PointerGrabState::Move { button, .. } | PointerGrabState::Resize { button, .. } => {
                Some(button)
            }
        }
    }

    /// Window being manipulated, if any.
    pub fn window_id(&self) -> Option<WindowId> {
        match *self {
            PointerGrabState::None => None,
            PointerGrabState::Move { id, .. } | PointerGrabState::Resize { id, .. } => Some(id),
        }
    }
}

pub struct AerowmState {
    pub engine: ScriptEngine,
    pub is_running: bool,
    pub subscribers: Vec<UnixStream>,
    pub compositor_state: CompositorState,
    pub shm_state: ShmState,
    /// Output globals (wl_output + xdg_output) live here: dropping the
    /// state would unregister them, and per-client binds dispatch through
    /// it via `delegate_output!` (see handlers).
    #[allow(dead_code)]
    pub output_manager_state: OutputManagerState,
    pub xdg_shell_state: XdgShellState,
    // Protocol states below are never read directly: Smithay's delegate
    // macros dispatch through the globals on the DisplayHandle. The fields
    // must still be retained — dropping them would unregister the globals.
    #[allow(dead_code)]
    pub session_lock_state: smithay::wayland::session_lock::SessionLockManagerState,
    pub is_locked: bool,
    pub take_screenshot: bool,
    pub lock_surfaces: Vec<smithay::wayland::session_lock::LockSurface>,
    pub xdg_decoration_state: XdgDecorationState,
    #[allow(dead_code)]
    pub fractional_scale_state: FractionalScaleManagerState,
    #[allow(dead_code)]
    pub viewporter_state: ViewporterState,
    
    pub layer_shell_state: WlrLayerShellState,
    pub seat_state: SeatState<Self>,
    pub seat: Seat<Self>,
    #[cfg(feature = "xwayland")]
    pub xwayland_shell: XWaylandShellState,
    pub pointer_location: Point<f64, Logical>,

    pub workspaces: Vec<Workspace>,
    pub scratchpad: Vec<WindowId>,

    pub active_ws: usize,
    pub surfaces: HashMap<WindowId, ToplevelSurface>,
    /// Restored placement directives awaiting matching windows.
    /// Consumed first-match-first-served as clients (re)connect.
    pub pending_placements: Vec<PendingPlacement>,
    /// Set by the `Restart` IPC command; the main loop exits and `exec`s.
    pub restart_requested: bool,
    /// Owned Wayland listening socket (fd survives `exec` for handover).
    pub wl_socket: Option<WaylandSocketInfo>,
    /// Managed X11 windows (XWayland), keyed like Wayland ones.
    #[cfg(feature = "xwayland")]
    pub x11_surfaces: HashMap<WindowId, X11Surface>,
    /// Unmanaged override-redirect X11 windows (menus, tooltips): visible
    /// in the space but outside workspaces, focus and layouts.
    #[cfg(feature = "xwayland")]
    pub override_redirect: Vec<X11Surface>,
    /// XWayland window manager instance, once the X server is ready.
    #[cfg(feature = "xwayland")]
    pub xwm: Option<X11Wm>,
    /// Event-loop handle kept for XWayland startup on `Ready`.
    #[cfg(feature = "xwayland")]
    pub loop_handle: Option<LoopHandle<'static, Self>>,

    /// Active pointer move/resize grab, if any.
    pub grab: PointerGrabState,
    /// Pinned user geometry of floating windows (source of truth for
    /// drag/resize and workspace remaps).
    pub float_geo: HashMap<WindowId, CoreRect>,

    pub space: Space<Window>,
    pub output: Option<Output>,
    pub damage_tracker: Option<OutputDamageTracker>,
    pub backend: Option<WinitGraphicsBackend<GlesRenderer>>,
    pub udev_data: Option<UdevRuntime>,
    pub popup_manager: PopupManager,
    pub output_size: Option<Size<i32, smithay::utils::Physical>>,
}

impl AerowmState {
    pub fn new(display_handle: &DisplayHandle, engine: ScriptEngine) -> Self {
        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(display_handle, "aerowm-seat");
        seat.add_keyboard(XkbConfig::default(), 200, 25)
            .expect("Failed to add keyboard to seat");
        seat.add_pointer();

        let workspaces = (1..=NUM_WORKSPACES)
            .map(|i| Workspace::new(i.to_string()))
            .collect();

        Self {
            engine,
            is_running: true,
            subscribers: Vec::new(),
            compositor_state: CompositorState::new::<Self>(display_handle),
            shm_state: ShmState::new::<Self>(display_handle, vec![]),
            output_manager_state: OutputManagerState::new_with_xdg_output::<Self>(
                display_handle,
            ),
            xdg_shell_state: XdgShellState::new::<Self>(display_handle),
            session_lock_state: smithay::wayland::session_lock::SessionLockManagerState::new::<Self, _>(display_handle, |_| true),
            is_locked: false,
            take_screenshot: false,
            lock_surfaces: Vec::new(),
            xdg_decoration_state: XdgDecorationState::new::<Self>(display_handle),
            fractional_scale_state: FractionalScaleManagerState::new::<Self>(display_handle),
            viewporter_state: ViewporterState::new::<Self>(display_handle),
            
            layer_shell_state: WlrLayerShellState::new::<Self>(display_handle),
            seat_state,
            seat,
            #[cfg(feature = "xwayland")]
            xwayland_shell: XWaylandShellState::new::<Self>(display_handle),
            pointer_location: Point::from((0.0, 0.0)),
            workspaces,
            scratchpad: Vec::new(),
            active_ws: 0,
            surfaces: HashMap::new(),
            pending_placements: Vec::new(),
            restart_requested: false,
            wl_socket: None,
            #[cfg(feature = "xwayland")]
            x11_surfaces: HashMap::new(),
            #[cfg(feature = "xwayland")]
            override_redirect: Vec::new(),
            #[cfg(feature = "xwayland")]
            xwm: None,
            #[cfg(feature = "xwayland")]
            loop_handle: None,
            grab: PointerGrabState::None,
            float_geo: HashMap::new(),
            space: Space::default(),
            output: None,
            damage_tracker: None,
            backend: None,
            udev_data: None,
            popup_manager: PopupManager::default(),
            output_size: None,
        }
    }

    pub fn active_workspace(&self) -> &Workspace {
        &self.workspaces[self.active_ws]
    }

    pub fn active_workspace_mut(&mut self) -> &mut Workspace {
        &mut self.workspaces[self.active_ws]
    }

    /// Builds a serializable snapshot for status clients (`aerowm-bar`).
    pub fn snapshot(&self) -> aerowm_ipc::CompositorSnapshot {
        let workspaces = self
            .workspaces
            .iter()
            .enumerate()
            .map(|(idx, ws)| aerowm_ipc::WorkspaceInfo {
                name: ws.name.clone(),
                active: idx == self.active_ws,
                window_count: ws.get_windows().len(),
                focused_window: ws.get_focused().map(|id| id.as_usize()),
            })
            .collect();
        aerowm_ipc::CompositorSnapshot {
            workspaces,
            active: self.active_ws,
            layout: self.active_workspace().get_current_layout().name().to_string(),
        }
    }

    /// Broadcasts a live event to all connected Pub/Sub IPC clients (e.g. status bars).
    pub fn broadcast_event(&mut self, event: IpcEvent) {
        if self.subscribers.is_empty() {
            return;
        }
        if let Ok(mut payload) = serde_json::to_string(&event) {
            payload.push('\n'); // newline delimited JSON

            // Send payload, removing any subscriber that disconnected (write failed)
            self.subscribers.retain_mut(|stream| {
                stream.write_all(payload.as_bytes()).is_ok()
            });
        }
    }

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
            if let Some(surface) = self.surfaces.get(&window_id) {
                // Send the new size to the Wayland client
                surface.with_pending_state(|state| {
                    state.size = Some((rect.size.width as i32, rect.size.height as i32).into());
                });
                surface.send_configure();

                if let Some(window) = self.space.elements().find(|w| w.toplevel() == Some(surface)) {
                    let location = Point::from((rect.origin.x, rect.origin.y));
                    windows_to_move.push((window.clone(), location));
                }
            }
            #[cfg(feature = "xwayland")]
            if let Some(x11) = self.x11_surfaces.get(&window_id).cloned() {
                // X11 windows get their tile geometry through configure.
                let geo = smithay::utils::Rectangle::new(
                    (rect.origin.x, rect.origin.y).into(),
                    (rect.size.width as i32, rect.size.height as i32).into(),
                );
                if let Err(e) = x11.configure(Some(geo)) {
                    tracing::warn!("failed to configure X11 window: {e:?}");
                }
                if let Some(window) = self.window_object(window_id) {
                    let location = Point::from((rect.origin.x, rect.origin.y));
                    windows_to_move.push((window, location));
                }
            }
        }

        for (window, location) in windows_to_move {
            self.space.map_element(window, location, true);
        }
    }

    /// Clean up dead windows (closed by client)
    pub fn cleanup_dead_windows(&mut self) {
        let dead_window_ids: Vec<_> = self.space.elements()
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
        let dead_windows: Vec<_> = self.space.elements()
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

    /// Returns the wl_surface of the currently focused window, if any.
    pub fn focused_surface(&self) -> Option<ToplevelSurface> {
        self.active_workspace()
            .get_focused()
            .and_then(|id| self.surfaces.get(&id).cloned())
    }

    /// Layer surface with exclusive keyboard grab (lock screens, launcher
    /// popups), if any. It takes precedence over tiled windows.
    fn exclusive_layer_focus(&self) -> Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface> {
        use smithay::wayland::shell::wlr_layer::KeyboardInteractivity;
        for output in self.space.outputs() {
            let map = smithay::desktop::layer_map_for_output(output);
            for layer in map.layers() {
                if layer.can_receive_keyboard_focus()
                    && layer.cached_state().keyboard_interactivity
                        == KeyboardInteractivity::Exclusive
                {
                    return Some(layer.wl_surface().clone());
                }
            }
        }
        None
    }

    /// Syncs Smithay keyboard focus + window activation with the workspace focus.
    pub fn update_keyboard_focus(&mut self) {
        let serial = SERIAL_COUNTER.next_serial();

        // Exclusive layer surfaces (lock screens) steal all keyboard input.
        if let Some(surface) = self.exclusive_layer_focus() {
            for window in self.space.elements() {
                window.set_activated(false);
            }
            let seat = self.seat.clone();
            if let Some(keyboard) = seat.get_keyboard() {
                keyboard.set_focus(self, Some(surface), serial);
            }
            return;
        }

        let focused_id = self.active_workspace().get_focused();
        let focused = self.focused_wl_surface();

        // Activate the focused window, deactivate the rest. Identity runs
        // through workspace ids so Wayland and X11 windows share the path.
        for window in self.space.elements() {
            let is_focused = focused_id
                .map(|fid| Some(fid) == self.id_of_window(window))
                .unwrap_or(false);
            window.set_activated(is_focused);
        }

        let seat = self.seat.clone();
        if let Some(keyboard) = seat.get_keyboard() {
            keyboard.set_focus(self, focused, serial);
        }

        if let Some(id) = self.active_workspace().get_focused() {
            self.broadcast_event(IpcEvent::FocusChanged(Some(id.as_usize())));
        }
    }

    /// Focuses a window by id (Wayland or X11). No-op for unknown ids.
    pub fn focus_window_id(&mut self, id: WindowId) {
        self.active_workspace_mut().focus_window(id);
        self.update_keyboard_focus();
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

    /// Closes the currently focused window (`kill_active`).
    pub fn kill_active(&mut self) {
        if let Some(surface) = self.focused_surface() {
            surface.send_close();
            return;
        }
        #[cfg(feature = "xwayland")]
        if let Some(x11) = self.focused_x11_surface()
            && let Err(e) = x11.close()
        {
            tracing::warn!("failed to close X11 window: {e:?}");
        }
    }

    /// Focused X11 surface, if the focused window is an X11 one.
    #[cfg(feature = "xwayland")]
    pub fn focused_x11_surface(&self) -> Option<X11Surface> {
        self.active_workspace()
            .get_focused()
            .and_then(|id| self.x11_surfaces.get(&id).cloned())
    }

    /// `wl_surface` of the focused window, whether Wayland or X11.
    /// `None` when nothing (committed yet) is focused.
    pub fn focused_wl_surface(
        &self,
    ) -> Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface> {
        if let Some(surface) = self.focused_surface() {
            return Some(surface.wl_surface().clone());
        }
        #[cfg(feature = "xwayland")]
        if let Some(x11) = self.focused_x11_surface() {
            return x11.wl_surface();
        }
        None
    }

    /// Fully removes a managed X11 window: workspace, maps, geometry,
    /// grabs and space. Idempotent — safe to call from both unmap and
    /// destroy notifications.
    #[cfg(feature = "xwayland")]
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

    pub fn focus_next(&mut self) {
        self.active_workspace_mut().focus_next();
        self.update_keyboard_focus();
    }

    pub fn focus_prev(&mut self) {
        self.active_workspace_mut().focus_prev();
        self.update_keyboard_focus();
    }

    pub fn swap_master(&mut self) {
        self.active_workspace_mut().swap_master();
        self.apply_layout();
    }

    /// Window object currently mapped for a window id, if any.
    fn window_object(&self, id: WindowId) -> Option<Window> {
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
    fn float_window(&mut self, id: WindowId) -> bool {
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
        let ws_idx = entry
            .workspace
            .min(self.workspaces.len().saturating_sub(1));
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
                self.surfaces.get(&id).cloned().map(Window::new_wayland_window)
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
                    if let Some(pinned) = self.float_geo.get(&id).copied() {
                        self.configure_window_size(id, &pinned);
                    }
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

    /// Starts an interactive move. The window is floated first (pinning its
    /// geometry) and the rest of the workspace re-tiles around the gap.
    pub fn begin_move_grab(&mut self, id: WindowId, button: u32, cursor: Point<f64, Logical>) {
        if self.float_window(id) {
            self.apply_layout();
        }
        let Some(pinned) = self.float_geo.get(&id).copied() else {
            self.grab = PointerGrabState::None;
            return;
        };
        self.active_workspace_mut().focus_window(id);
        self.update_keyboard_focus();
        self.grab = PointerGrabState::Move {
            id,
            button,
            dx: cursor.x - pinned.origin.x as f64,
            dy: cursor.y - pinned.origin.y as f64,
        };
    }

    /// Starts an interactive resize from the corner nearest to the cursor.
    pub fn begin_resize_grab(&mut self, id: WindowId, button: u32, cursor: Point<f64, Logical>) {
        if self.float_window(id) {
            self.apply_layout();
        }
        let Some(pinned) = self.float_geo.get(&id).copied() else {
            self.grab = PointerGrabState::None;
            return;
        };
        let corner = nearest_corner(&pinned, cursor.x, cursor.y);
        self.active_workspace_mut().focus_window(id);
        self.update_keyboard_focus();
        self.grab = PointerGrabState::Resize {
            id,
            button,
            corner,
            start: pinned,
            cursor_x: cursor.x,
            cursor_y: cursor.y,
        };
    }

    /// Advances the active grab to the cursor position. No-op without a grab.
    pub fn update_grab_to(&mut self, cursor_x: f64, cursor_y: f64) {
        match self.grab {
            PointerGrabState::None => {}
            PointerGrabState::Move { id, dx, dy, .. } => {
                let nx = (cursor_x - dx).round() as i32;
                let ny = (cursor_y - dy).round() as i32;
                if let Some(window) = self.window_object(id) {
                    self.space.map_element(window, Point::from((nx, ny)), false);
                }
                if let Some(geo) = self.float_geo.get_mut(&id) {
                    geo.origin.x = nx;
                    geo.origin.y = ny;
                }
            }
            PointerGrabState::Resize {
                id,
                corner,
                start,
                cursor_x: sx,
                cursor_y: sy,
                ..
            } => {
                let dx = (cursor_x - sx).round() as i32;
                let dy = (cursor_y - sy).round() as i32;
                let x1 = start.origin.x + start.size.width as i32;
                let y1 = start.origin.y + start.size.height as i32;
                let (nx, ny, nw, nh) = match corner {
                    GrabCorner::TopLeft => {
                        let nx = (start.origin.x + dx).min(x1 - MIN_FLOAT_W);
                        let ny = (start.origin.y + dy).min(y1 - MIN_FLOAT_H);
                        (nx, ny, x1 - nx, y1 - ny)
                    }
                    GrabCorner::TopRight => {
                        let ny = (start.origin.y + dy).min(y1 - MIN_FLOAT_H);
                        let nw = (start.size.width as i32 + dx).max(MIN_FLOAT_W);
                        (start.origin.x, ny, nw, y1 - ny)
                    }
                    GrabCorner::BottomLeft => {
                        let nx = (start.origin.x + dx).min(x1 - MIN_FLOAT_W);
                        let nh = (start.size.height as i32 + dy).max(MIN_FLOAT_H);
                        (nx, start.origin.y, x1 - nx, nh)
                    }
                    GrabCorner::BottomRight => {
                        let nw = (start.size.width as i32 + dx).max(MIN_FLOAT_W);
                        let nh = (start.size.height as i32 + dy).max(MIN_FLOAT_H);
                        (start.origin.x, start.origin.y, nw, nh)
                    }
                };
                if let Some(window) = self.window_object(id) {
                    self.space.map_element(window, Point::from((nx, ny)), false);
                }
                let resized = CoreRect::new(nx, ny, nw as u32, nh as u32);
                if let Some(surface) = self.surfaces.get(&id) {
                    surface.with_pending_state(|s| {
                        s.size = Some((nw, nh).into());
                    });
                    surface.send_configure();
                }
                #[cfg(feature = "xwayland")]
                if !self.surfaces.contains_key(&id)
                    && let Some(x11) = self.x11_surfaces.get(&id)
                {
                    let geo = smithay::utils::Rectangle::new((nx, ny).into(), (nw, nh).into());
                    if let Err(e) = x11.configure(Some(geo)) {
                        tracing::warn!("failed to configure X11 window: {e:?}");
                    }
                }
                self.float_geo.insert(id, resized);
            }
        }
    }

    pub fn end_grab(&mut self) {
        self.grab = PointerGrabState::None;
    }

    pub fn spawn(&self, cmd: &str) {
        let _ = std::process::Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .spawn();
    }

    fn remap_active_workspace(&mut self) {
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
                if let Some(pinned) = self.float_geo.get(&id).copied() {
                    self.configure_window_size(id, &pinned);
                }
                self.space.map_element(window, loc, false);
            } else {
                // Re-map; real position comes from apply_layout right after.
                self.space.map_element(window, (0, 0), false);
            }
        }
        self.apply_layout();
        self.update_keyboard_focus();
    }

    /// Pushes a size configure to a managed window on either backend.
    fn configure_window_size(&self, id: WindowId, rect: &CoreRect) {
        if let Some(surface) = self.surfaces.get(&id) {
            surface.with_pending_state(|s| {
                s.size = Some((rect.size.width as i32, rect.size.height as i32).into());
            });
            surface.send_configure();
        }
        #[cfg(feature = "xwayland")]
        if !self.surfaces.contains_key(&id)
            && let Some(x11) = self.x11_surfaces.get(&id)
        {
            let geo = smithay::utils::Rectangle::new(
                (rect.origin.x, rect.origin.y).into(),
                (rect.size.width as i32, rect.size.height as i32).into(),
            );
            if let Err(e) = x11.configure(Some(geo)) {
                tracing::warn!("failed to configure X11 window: {e:?}");
            }
        }
    }


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
        
                
        if let Some(scratchpad) = rules.scratchpad {
            if scratchpad && !self.scratchpad.contains(&id) {
                self.scratchpad.push(id);
                for ws in &mut self.workspaces {
                    ws.remove_window(id);
                }
                if let Some(window) = self.window_object(id) {
                    self.space.unmap_elem(&window);
                }
                self.float_window(id);
            }
        }
if let Some(floating) = rules.floating {
            if floating {
                self.float_window(id);
            }
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

    /// Performs an atomic Hot Reload of the Luau configuration.
    pub fn reload_config(&mut self) -> Result<(), String> {
        let new_engine = aerowm_lua::ScriptEngine::new().map_err(|e| format!("Failed to init new Lua engine: {}", e))?;
        new_engine.load_default_config().map_err(|e| format!("Config syntax error: {}", e))?;
        
        self.engine = new_engine;
        tracing::info!("Hot reload successful");
        let _ = self.engine.emit_hook("reload");
        Ok(())
    }

    pub fn switch_workspace(&mut self, idx: usize) {
        if idx >= self.workspaces.len() {
            tracing::warn!(
                "workspace switch to index {idx} ignored, only {} workspaces",
                self.workspaces.len()
            );
            return;
        }
        if idx == self.active_ws {
            return;
        }
        self.active_ws = idx;
        self.remap_active_workspace();
        self.broadcast_event(IpcEvent::WorkspaceSwitched(
            self.active_workspace().name.clone(),
        ));
    }

    pub fn next_workspace(&mut self) {
        let next = (self.active_ws + 1) % self.workspaces.len();
        self.switch_workspace(next);
    }

    pub fn prev_workspace(&mut self) {
        let prev = if self.active_ws == 0 {
            self.workspaces.len() - 1
        } else {
            self.active_ws - 1
        };
        self.switch_workspace(prev);
    }

    /// Starts the XWayland server and, once ready, the X11 window manager.
    ///
    /// Missing `Xwayland` binary (or any other startup failure) is reported
    /// as `Err` so the caller can keep running Wayland-only.
    #[cfg(feature = "xwayland")]
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

    pub fn toggle_scratchpad(&mut self) {
        if self.scratchpad.is_empty() {
            return;
        }

        let mut focused_sp_idx = None;
        if let Some(focused) = self.active_workspace().get_focused() {
            if let Some(pos) = self.scratchpad.iter().position(|&w| w == focused) {
                focused_sp_idx = Some(pos);
            }
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

}

/// Corner of a rectangle nearest to the cursor position.
fn nearest_corner(rect: &CoreRect, cursor_x: f64, cursor_y: f64) -> GrabCorner {
    let x0 = rect.origin.x as f64;
    let y0 = rect.origin.y as f64;
    let x1 = x0 + rect.size.width as f64;
    let y1 = y0 + rect.size.height as f64;
    let left = (cursor_x - x0).abs() < (cursor_x - x1).abs();
    let top = (cursor_y - y0).abs() < (cursor_y - y1).abs();
    match (top, left) {
        (true, true) => GrabCorner::TopLeft,
        (true, false) => GrabCorner::TopRight,
        (false, true) => GrabCorner::BottomLeft,
        (false, false) => GrabCorner::BottomRight,
    }
}

/// Default `OutputHandler`: geometry/mode/done on bind are sent by
/// Smithay's `OutputManagerState`; we track nothing extra per bind.
impl OutputHandler for AerowmState {}
