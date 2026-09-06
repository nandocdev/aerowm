use aerowm_lua::ScriptEngine;
use wayland_server::DisplayHandle;
use std::os::unix::net::UnixStream;
use std::io::Write;
use aerowm_ipc::IpcEvent;

use smithay::wayland::compositor::CompositorState;
use smithay::wayland::shm::ShmState;
use smithay::wayland::shell::xdg::{XdgShellState, ToplevelSurface};
use smithay::input::SeatState;
use smithay::desktop::space::Space;
use smithay::desktop::Window;
use smithay::desktop::PopupManager;
use smithay::output::Output;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::WinitGraphicsBackend;
use smithay::utils::{Point, Size, IsAlive};
use smithay::wayland::output::WlOutputData;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::GlobalDispatch;

use std::collections::HashMap;
use aerowm_core::id::WindowId;
use aerowm_core::workspace::Workspace;
use aerowm_core::geometry::Rect as CoreRect;

pub struct AerowmState {
    pub engine: ScriptEngine,
    pub is_running: bool,
    pub subscribers: Vec<UnixStream>,
    pub compositor_state: CompositorState,
    pub shm_state: ShmState,
    pub xdg_shell_state: XdgShellState,
    pub seat_state: SeatState<Self>,

    pub active_workspace: Workspace,
    pub surfaces: HashMap<WindowId, ToplevelSurface>,

    pub space: Space<Window>,
    pub output: Option<Output>,
    pub damage_tracker: Option<OutputDamageTracker>,
    pub backend: Option<WinitGraphicsBackend<GlesRenderer>>,
    pub popup_manager: PopupManager,
    pub output_size: Option<Size<i32, smithay::utils::Physical>>,
}

impl AerowmState {
    pub fn new(display_handle: &DisplayHandle, engine: ScriptEngine) -> Self {
        let mut seat_state = SeatState::new();
        let _seat = seat_state.new_wl_seat(display_handle, "aerowm-seat");
        Self {
            engine,
            is_running: true,
            subscribers: Vec::new(),
            compositor_state: CompositorState::new::<Self>(display_handle),
            shm_state: ShmState::new::<Self>(display_handle, vec![]),
            xdg_shell_state: XdgShellState::new::<Self>(display_handle),
            seat_state,
            active_workspace: Workspace::new("1"),
            surfaces: HashMap::new(),
            space: Space::default(),
            output: None,
            damage_tracker: None,
            backend: None,
            popup_manager: PopupManager::default(),
            output_size: None,
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

    /// Apply the current layout to all windows in the active workspace
    pub fn apply_layout(&mut self) {
        let Some(output_size) = self.output_size else {
            return;
        };
        
        let area = CoreRect::new(0, 0, output_size.w as u32, output_size.h as u32);
        let layout = self.active_workspace.get_current_layout();
        let window_rects = self.active_workspace.apply_layout(layout, area);
        
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
            self.active_workspace.remove_window(id);
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