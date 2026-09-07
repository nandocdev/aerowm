use aerowm_lua::ScriptEngine;
use wayland_server::DisplayHandle;
use std::os::unix::net::UnixStream;
use std::io::Write;
use aerowm_ipc::IpcEvent;

use smithay::wayland::compositor::CompositorState;
use smithay::wayland::shm::ShmState;
use smithay::wayland::shell::wlr_layer::WlrLayerShellState;
use smithay::wayland::shell::xdg::{XdgShellState, ToplevelSurface};
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
use smithay::wayland::output::WlOutputData;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::GlobalDispatch;

use std::collections::HashMap;
use aerowm_core::id::WindowId;
use aerowm_core::workspace::Workspace;
use aerowm_core::geometry::Rect as CoreRect;

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
    pub xdg_shell_state: XdgShellState,
    pub layer_shell_state: WlrLayerShellState,
    pub seat_state: SeatState<Self>,
    pub seat: Seat<Self>,
    pub pointer_location: Point<f64, Logical>,

    pub workspaces: Vec<Workspace>,
    pub active_ws: usize,
    pub surfaces: HashMap<WindowId, ToplevelSurface>,

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
            xdg_shell_state: XdgShellState::new::<Self>(display_handle),
            layer_shell_state: WlrLayerShellState::new::<Self>(display_handle),
            seat_state,
            seat,
            pointer_location: Point::from((0.0, 0.0)),
            workspaces,
            active_ws: 0,
            surfaces: HashMap::new(),
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
        }

        for (window, location) in windows_to_move {
            self.space.map_element(window, location, true);
        }
    }

    /// Clean up dead windows (closed by client)
    pub fn cleanup_dead_windows(&mut self) {
        let dead_window_ids: Vec<_> = self.space.elements()
            .filter(|w| !w.alive())
            .filter_map(|w| w.toplevel().cloned())
            .filter_map(|surface| {
                self.surfaces.iter().find(|(_, s)| **s == surface).map(|(id, _)| *id)
            })
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

        let focused = self.focused_surface();

        // Activate the focused window, deactivate the rest.
        for window in self.space.elements() {
            let is_focused = focused
                .as_ref()
                .map(|s| window.toplevel() == Some(s))
                .unwrap_or(false);
            window.set_activated(is_focused);
        }

        let seat = self.seat.clone();
        if let Some(keyboard) = seat.get_keyboard() {
            let focus = focused.map(|s| s.wl_surface().clone());
            keyboard.set_focus(self, focus, serial);
        }

        if let Some(id) = self.active_workspace().get_focused() {
            self.broadcast_event(IpcEvent::FocusChanged(Some(id.as_usize())));
        }
    }

    /// Focuses the window containing the given toplevel surface.
    pub fn focus_toplevel(&mut self, surface: &ToplevelSurface) {
        if let Some((id, _)) = self.surfaces.iter().find(|(_, s)| *s == surface).map(|(id, _)| (*id, ())) {
            self.active_workspace_mut().focus_window(id);
            self.update_keyboard_focus();
        }
    }

    /// Closes the currently focused window (`kill_active`).
    pub fn kill_active(&mut self) {
        if let Some(surface) = self.focused_surface() {
            surface.send_close();
        }
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
        let surface = self.surfaces.get(&id)?;
        self.space
            .elements()
            .find(|w| w.toplevel() == Some(surface))
            .cloned()
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
                if let Some(surface) = self.surfaces.get(&id).cloned() {
                    surface.with_pending_state(|s| {
                        s.size = Some((nw, nh).into());
                    });
                    surface.send_configure();
                }
                self.float_geo
                    .insert(id, CoreRect::new(nx, ny, nw as u32, nh as u32));
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
        let mut by_id: HashMap<WindowId, Window> = HashMap::new();
        for window in self.space.elements() {
            if let Some(toplevel) = window.toplevel() {
                if let Some((id, _)) = self
                    .surfaces
                    .iter()
                    .find(|(_, s)| *s == toplevel)
                {
                    by_id.insert(*id, window.clone());
                }
            }
        }
        let all: Vec<Window> = self.space.elements().cloned().collect();
        for w in all {
            self.space.unmap_elem(&w);
        }
        let ids: Vec<WindowId> = self.active_workspace().get_windows().to_vec();
        for id in ids {
            if let Some(surface) = self.surfaces.get(&id).cloned() {
                let window = by_id
                    .remove(&id)
                    .unwrap_or_else(|| Window::new_wayland_window(surface.clone()));
                if self.active_workspace().is_floating(id) {
                    // Floating windows keep their pinned user geometry.
                    let loc = self
                        .float_geo
                        .get(&id)
                        .map(|r| Point::from((r.origin.x, r.origin.y)))
                        .unwrap_or(Point::from((0, 0)));
                    if let Some(pinned) = self.float_geo.get(&id) {
                        surface.with_pending_state(|s| {
                            s.size = Some(
                                (pinned.size.width as i32, pinned.size.height as i32).into(),
                            );
                        });
                        surface.send_configure();
                    }
                    self.space.map_element(window, loc, false);
                } else {
                    // Re-map; real position comes from apply_layout right after.
                    self.space.map_element(window, (0, 0), false);
                }
            }
        }
        self.apply_layout();
        self.update_keyboard_focus();
    }

    pub fn switch_workspace(&mut self, idx: usize) {
        if idx >= self.workspaces.len() || idx == self.active_ws {
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

impl GlobalDispatch<WlOutput, WlOutputData> for AerowmState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &wayland_server::Client,
        _resource: wayland_server::New<WlOutput>,
        _global_data: &WlOutputData,
        _data_init: &mut wayland_server::DataInit<'_, Self>,
    ) {
        // Output binding is handled by the Output::create_global
    }
}
