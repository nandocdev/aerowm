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
                    .unwrap_or_else(|| Window::new_wayland_window(surface));
                // Re-map; real position comes from apply_layout right after.
                self.space.map_element(window, (0, 0), false);
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
