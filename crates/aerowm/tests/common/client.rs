//! Minimal in-process Wayland client for integration tests.
//!
//! Raw `wayland-protocols` objects (no toolkit): registry → compositor +
//! xdg_wm_base → surface/toplevel, with ping/configure handled so the
//! server-side handshake completes.

use wayland_client::protocol::{
    wl_compositor::WlCompositor, wl_registry::WlRegistry, wl_surface::WlSurface,
};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols::xdg::shell::client::{
    xdg_surface::XdgSurface, xdg_toplevel::XdgToplevel, xdg_wm_base::XdgWmBase,
};

pub struct Env {
    pub globals: Vec<(u32, String, u32)>,
    pub surfaces: Vec<WlSurface>,
    pub close_seen: bool,
    /// Every xdg_toplevel configure size, in arrival order.
    pub configures: Vec<(i32, i32)>,
}

impl Env {
    fn find(&self, interface: &str) -> Option<(u32, u32)> {
        self.globals
            .iter()
            .find(|(_, i, _)| i == interface)
            .map(|(n, _, v)| (*n, *v))
    }
}

// --- Dispatch impls (all no-op except registry/ping/configure/close) --------

impl Dispatch<WlRegistry, ()> for Env {
    fn event(
        state: &mut Self,
        _: &WlRegistry,
        event: wayland_client::protocol::wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wayland_client::protocol::wl_registry::Event::*;
        match event {
            Global { name, interface, version } => {
                state.globals.push((name, interface, version))
            }
            GlobalRemove { name } => state.globals.retain(|(n, _, _)| *n != name),
            _ => {}
        }
    }
}

impl Dispatch<WlCompositor, ()> for Env {
    fn event(
        _: &mut Self,
        _: &WlCompositor,
        _: <WlCompositor as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSurface, ()> for Env {
    fn event(
        _: &mut Self,
        _: &WlSurface,
        _: <WlSurface as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<XdgWmBase, ()> for Env {
    fn event(
        _: &mut Self,
        proxy: &XdgWmBase,
        event: <XdgWmBase as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wayland_protocols::xdg::shell::client::xdg_wm_base::Event::Ping;
        if let Ping { serial } = event {
            proxy.pong(serial);
        }
    }
}

impl Dispatch<XdgSurface, ()> for Env {
    fn event(
        state: &mut Self,
        proxy: &XdgSurface,
        event: <XdgSurface as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wayland_protocols::xdg::shell::client::xdg_surface::Event::Configure;
        if let Configure { serial } = event {
            proxy.ack_configure(serial);
            // Commit everything we know: harmless and avoids tracking
            // which surface each configure belongs to.
            for s in &state.surfaces {
                s.commit();
            }
        }
    }
}

impl Dispatch<XdgToplevel, ()> for Env {
    fn event(
        state: &mut Self,
        _: &XdgToplevel,
        event: <XdgToplevel as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wayland_protocols::xdg::shell::client::xdg_toplevel::Event::*;
        match event {
            Close => state.close_seen = true,
            Configure { width, height, .. } => state.configures.push((width, height)),
            ConfigureBounds { .. } | WmCapabilities { .. } => {}
            _ => {}
        }
    }
}

// --- Client ------------------------------------------------------------------

pub struct Win {
    pub surface: WlSurface,
    pub xdg_surface: XdgSurface,
    pub toplevel: XdgToplevel,
}

impl Win {
    pub fn destroy(self, client: &mut TestClient) {
        self.toplevel.destroy();
        self.xdg_surface.destroy();
        self.surface.destroy();
        client
            .env
            .surfaces
            .retain(|s| s != &self.surface);
        client.settle();
    }
}

pub struct TestClient {
    queue: EventQueue<Env>,
    qh: QueueHandle<Env>,
    compositor: WlCompositor,
    wm_base: XdgWmBase,
    pub env: Env,
}

impl TestClient {
    /// Connects over an already-inserted socketpair end and binds globals.
    pub fn connect(stream: std::os::unix::net::UnixStream) -> Self {
        let conn = Connection::from_socket(stream).expect("client connect");
        let mut queue = conn.new_event_queue::<Env>();
        let qh = queue.handle();
        let display = conn.display();
        let registry = display.get_registry(&qh, ());
        let mut env = Env {
            globals: Vec::new(),
            surfaces: Vec::new(),
            close_seen: false,
            configures: Vec::new(),
        };
        queue.roundtrip(&mut env).expect("registry roundtrip");
        assert!(!env.globals.is_empty(), "server advertised no globals");

        let (name, version) = env.find("wl_compositor").expect("wl_compositor missing");
        let compositor: WlCompositor =
            registry.bind::<WlCompositor, (), Env>(name, version.min(6), &qh, ());
        let (name, version) = env.find("xdg_wm_base").expect("xdg_wm_base missing");
        let wm_base: XdgWmBase =
            registry.bind::<XdgWmBase, (), Env>(name, version.min(3), &qh, ());
        queue.roundtrip(&mut env).expect("bind roundtrip");

        Self { queue, qh, compositor, wm_base, env }
    }

    /// Creates, configures and commits a toplevel; pumps until the
    /// server-side handshake settles. Returns its proxies.
    pub fn map_window(&mut self, app_id: &str, title: &str) -> Win {
        let surface = self.compositor.create_surface(&self.qh, ());
        let xdg_surface = self.wm_base.get_xdg_surface(&surface, &self.qh, ());
        let toplevel = xdg_surface.get_toplevel(&self.qh, ());
        toplevel.set_app_id(app_id.to_string());
        toplevel.set_title(title.to_string());
        surface.commit();
        self.env.surfaces.push(surface.clone());
        self.settle();
        Win { surface, xdg_surface, toplevel }
    }

    /// Pumps the client side until the server handshake settles
    /// (configure → ack → commit roundtrips).
    pub fn settle(&mut self) {
        for _ in 0..4 {
            self.queue.roundtrip(&mut self.env).expect("settle roundtrip");
        }
    }

    /// One blocking pump step (terminates via wl_display.sync).
    pub fn pump(&mut self) {
        self.queue.roundtrip(&mut self.env).expect("pump roundtrip");
    }
}
