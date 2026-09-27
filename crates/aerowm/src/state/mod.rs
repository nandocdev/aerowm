//! The compositor state, split by responsibility across submodules.
//!
//! Smithay's `delegate_*!` macros expand to `impl XyzHandler for
//! AerowmState` and need a single concrete type owning every protocol
//! state, so [`AerowmState`] stays one struct. What can be split is the
//! logic hanging off it, and that is what lives here: each submodule owns
//! one area, so a focus change no longer has to scroll past resize
//! arithmetic and XWayland lifecycle to find its three lines.
//!
//! - [`focus`]: keyboard focus, activation and workspace navigation
//! - [`grab`]: interactive pointer move/resize grabs
//! - [`layout`]: tiling, space mapping and floating geometry
//! - [`window`]: per-window bookkeeping (floating, scratchpad, cleanup)
//! - [`surface`]: backend-agnostic window lookup
//! - [`rules`]: declarative Luau window rules
//! - [`session`]: session restore and config hot reload
//! - [`ipc`]: status snapshots and event broadcast
//! - `xwayland` (feature `xwayland`): XWayland startup and X11 window removal

mod focus;
mod grab;
mod ipc;
mod layout;
mod rules;
mod session;
mod surface;
mod window;
#[cfg(feature = "xwayland")]
mod xwayland;

use aerowm_lua::ScriptEngine;
use std::os::unix::net::UnixStream;
use wayland_server::DisplayHandle;

use smithay::wayland::compositor::CompositorState;
use smithay::wayland::fractional_scale::FractionalScaleManagerState;
use smithay::wayland::shell::wlr_layer::WlrLayerShellState;
use smithay::wayland::shell::xdg::decoration::XdgDecorationState;
use smithay::wayland::shell::xdg::{ToplevelSurface, XdgShellState};
use smithay::wayland::shm::ShmState;
use smithay::wayland::viewporter::ViewporterState;

use crate::backend::udev::UdevRuntime;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::WinitGraphicsBackend;
use smithay::desktop::PopupManager;
use smithay::desktop::Window;
use smithay::desktop::space::Space;
use smithay::input::{Seat, SeatState, keyboard::XkbConfig};
use smithay::output::Output;
use smithay::utils::{Logical, Point, Size};
use smithay::wayland::output::{OutputHandler, OutputManagerState};
#[cfg(feature = "xwayland")]
use smithay::wayland::xwayland_shell::XWaylandShellState;
#[cfg(feature = "xwayland")]
use smithay::xwayland::{X11Surface, X11Wm};

use crate::wayland_socket::WaylandSocketInfo;
use aerowm_core::geometry::Rect as CoreRect;
use aerowm_core::id::WindowId;
use aerowm_core::session::PendingPlacement;
use aerowm_core::workspace::Workspace;
use std::collections::HashMap;

pub use grab::{GrabCorner, PointerGrabState};

pub const NUM_WORKSPACES: usize = 4;

/// Minimum user-resizable size for floating windows.
pub const MIN_FLOAT_W: i32 = 120;
pub const MIN_FLOAT_H: i32 = 80;

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
    pub loop_handle: Option<calloop::LoopHandle<'static, Self>>,

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
            output_manager_state: OutputManagerState::new_with_xdg_output::<Self>(display_handle),
            xdg_shell_state: XdgShellState::new::<Self>(display_handle),
            session_lock_state: smithay::wayland::session_lock::SessionLockManagerState::new::<
                Self,
                _,
            >(display_handle, |_| true),
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

    /// Spawns a command detached from the compositor, for binds that
    /// launch a terminal. Failures are not fatal to the session.
    pub fn spawn(&self, cmd: &str) {
        let _ = std::process::Command::new("sh").arg("-c").arg(cmd).spawn();
    }
}

/// Default `OutputHandler`: geometry/mode/done on bind are sent by
/// Smithay's `OutputManagerState`; we track nothing extra per bind.
impl OutputHandler for AerowmState {}
