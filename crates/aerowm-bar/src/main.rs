//! `aerowm-bar`: layer-shell status bar for AeroWM.
//!
//! A `wlr-layer-shell` client (top bar, exclusive zone) whose widgets are
//! defined in Luau (`~/.config/aerowm/bar.luau`) and whose live data comes
//! over the AeroWM IPC socket (`GetState` query + event subscription).
//! Workspace cells are clickable (click → switch workspace via IPC).

mod config;
mod ipc_client;
mod render;

use std::time::Duration;

use calloop::timer::{TimeoutAction, Timer};
use calloop::{EventLoop, Interest, Mode, generic::Generic};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{Capability, SeatHandler, SeatState, pointer::{PointerEventKind, PointerHandler}},
    shell::{
        WaylandSurface,
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure},
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use wayland_client::{
    Connection, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
};

use aerowm_ipc::{IpcCommand, WorkspaceAction};

use config::BarConfig;
use render::{Canvas, WorkspaceCell, current_clock, draw_bar};

const NAMESPACE: &str = "aerowm-bar";
const BTN_LEFT: u32 = 0x110;

struct Bar {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    pool: Option<SlotPool>,
    layer: LayerSurface,
    width: u32,
    first_configure: bool,
    exit: bool,

    config: BarConfig,
    snapshot: aerowm_ipc::CompositorSnapshot,
    cells: Vec<WorkspaceCell>,
    clock_cache: String,
    ipc_sub: Option<ipc_client::Subscriber>,
}

impl Bar {
    fn draw(&mut self) {
        let width = self.width;
        let height = self.config.height;
        if width == 0 {
            return;
        }
        let stride = width as i32 * 4;
        let len = (width * height) as usize * 4;

        // (Re)create the pool when the width changed.
        let recreate = self
            .pool
            .as_ref()
            .map(|p| p.len() != len)
            .unwrap_or(true);
        if recreate {
            match SlotPool::new(len, &self.shm) {
                Ok(pool) => self.pool = Some(pool),
                Err(e) => {
                    log::error!("shm pool: {e:?}");
                    return;
                }
            }
        }
        let pool = self.pool.as_mut().expect("pool just created");
        let (buffer, canvas) = match pool.create_buffer(
            width as i32,
            height as i32,
            stride,
            wl_shm::Format::Argb8888,
        ) {
            Ok(v) => v,
            Err(e) => {
                log::error!("create buffer: {e:?}");
                return;
            }
        };

        let mut view = Canvas::new(canvas, width, height);
        self.cells = draw_bar(&mut view, &self.config, &self.snapshot, &self.clock_cache);

        self.layer
            .wl_surface()
            .damage_buffer(0, 0, width as i32, height as i32);
        if let Err(e) = buffer.attach_to(self.layer.wl_surface()) {
            log::error!("attach: {e:?}");
            return;
        }
        self.layer.commit();
    }

    fn refresh_state(&mut self) {
        match ipc_client::query_state() {
            Ok(snapshot) => {
                self.snapshot = snapshot;
                self.draw();
            }
            Err(e) => log::warn!("state refresh failed: {e}"),
        }
    }

    fn click(&mut self, x: f64) {
        let x = x as u32;
        if let Some(cell) = self.cells.iter().find(|c| x >= c.x0 && x < c.x1) {
            log::info!("workspace click → {}", cell.index + 1);
            // User-facing workspaces are 1-based.
            ipc_client::send_command(&IpcCommand::Workspace {
                action: WorkspaceAction::Switch(cell.index + 1),
            });
        }
    }
}

// --- Wayland delegates -------------------------------------------------------

impl CompositorHandler for Bar {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
    }
    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }
    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
    }
    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Bar {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _output: wl_output::WlOutput) {}
    fn update_output(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _output: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _output: wl_output::WlOutput) {}
}

impl SeatHandler for Bar {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {}
    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer
            && let Ok(pointer) = self.seat_state.get_pointer(qh, &seat)
        {
            log::info!("pointer capability acquired");
            let _ = pointer;
        }
    }
    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        _capability: Capability,
    ) {
    }
    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {}
}

impl PointerHandler for Bar {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[smithay_client_toolkit::seat::pointer::PointerEvent],
    ) {
        for event in events {
            if &event.surface != self.layer.wl_surface() {
                continue;
            }
            if let PointerEventKind::Press { button, .. } = event.kind
                && button == BTN_LEFT
            {
                self.click(event.position.0);
            }
        }
    }
}

impl LayerShellHandler for Bar {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        log::info!("layer surface closed, exiting");
        self.exit = true;
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        // The compositor suggests full output width; our height is fixed.
        if configure.new_size.0 != 0 {
            self.width = configure.new_size.0;
        }
        if self.first_configure {
            self.first_configure = false;
            layer.set_size(self.width, self.config.height);
            layer.commit();
        }
        self.draw();
    }
}

impl ShmHandler for Bar {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_compositor!(Bar);
delegate_output!(Bar);
delegate_shm!(Bar);
delegate_seat!(Bar);
delegate_pointer!(Bar);
delegate_layer!(Bar);
delegate_registry!(Bar);

impl ProvidesRegistryState for Bar {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

// --- Entry point ---------------------------------------------------------------

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let config = config::load();
    log::info!("bar config: height={} widgets={:?}", config.height, config.widgets);

    let conn = Connection::connect_to_env()?;
    let (globals, event_queue) = registry_queue_init::<Bar>(&conn)?;
    let qh = event_queue.handle();

    let compositor =
        CompositorState::bind(&globals, &qh).map_err(|_| "wl_compositor unavailable")?;
    let layer_shell = LayerShell::bind(&globals, &qh).map_err(|_| "wlr-layer-shell unavailable")?;
    let shm = Shm::bind(&globals, &qh).map_err(|_| "wl_shm unavailable")?;

    let surface = compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(
        &qh,
        surface,
        Layer::Top,
        Some(NAMESPACE),
        None,
    );
    layer.set_anchor(Anchor::TOP | Anchor::LEFT | Anchor::RIGHT);
    layer.set_exclusive_zone(config.height as i32);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.set_size(0, config.height);
    // Initial commit without a buffer → compositor answers with configure.
    layer.commit();

    let ipc_sub = match ipc_client::Subscriber::connect() {
        Ok(sub) => Some(sub),
        Err(e) => {
            log::warn!("no IPC subscription ({e}); relying on periodic refresh");
            None
        }
    };

    let mut bar = Bar {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool: None,
        layer,
        width: 0,
        first_configure: true,
        exit: false,
        config,
        snapshot: ipc_client::query_state().unwrap_or_default(),
        cells: Vec::new(),
        clock_cache: current_clock(),
        ipc_sub,
    };

    let mut event_loop: EventLoop<Bar> = EventLoop::try_new()?;

    // Wayland protocol events.
    {
        use calloop_wayland_source::WaylandSource;
        let source = WaylandSource::new(conn, event_queue);
        event_loop
            .handle()
            .insert_source(source, |_, queue, bar: &mut Bar| {
                queue.dispatch_pending(bar)
            })
            .map_err(|e| format!("wayland source: {e}"))?;
    }

    // Compositor state subscription → refresh + redraw on any event.
    // A cloned fd only provides readiness; reads go through the
    // `Subscriber` owned by `Bar` (same socket receive queue).
    if let Some(stream) = bar
        .ipc_sub
        .as_ref()
        .and_then(|s| s.stream().try_clone().ok())
    {
        let source = Generic::new(stream, Interest::READ, Mode::Level);
        event_loop
            .handle()
            .insert_source(source, |_, _, bar: &mut Bar| {
                let changed = bar.ipc_sub.as_mut().map(|s| s.drain_events()).unwrap_or(0);
                if changed > 0 {
                        bar.refresh_state();
                }
                Ok(calloop::PostAction::Continue)
            })
            .map_err(|e| format!("ipc source: {e}"))?;
    }

    // 1s tick: clock widget + periodic refresh backstop.
    event_loop
        .handle()
        .insert_source(Timer::from_duration(Duration::from_secs(1)), |_, _, bar: &mut Bar| {
            let now = current_clock();
            if now != bar.clock_cache {
                bar.clock_cache = now;
                bar.draw();
            }
            TimeoutAction::ToDuration(Duration::from_secs(1))
        })
        .map_err(|e| format!("timer: {e}"))?;

    while !bar.exit {
        event_loop.dispatch(None, &mut bar)?;
    }
    Ok(())
}
