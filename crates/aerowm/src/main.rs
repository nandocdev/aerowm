use tracing::{error, info, warn};
use calloop::EventLoop;
use wayland_server::Display;
use aerowm_lua::ScriptEngine;
use aerowm::state::AerowmState;
use aerowm::{backend, ipc, session, wayland_socket};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    info!("Starting AeroWM...");

    let mut event_loop: EventLoop<'static, AerowmState> = EventLoop::try_new()?;
    let mut display: Display<AerowmState> = Display::new()?;
    let display_handle = display.handle();

    info!("Initializing Luau Engine...");
    let engine = ScriptEngine::new().map_err(|e| format!("Failed to init ScriptEngine: {}", e))?;

    if let Err(err) = engine.load_config_string("aerowm.log('Luau loaded successfully!')") {
        warn!("Failed to load config: {}", err);
    }
    let _ = engine.emit_hook("startup");

    let mut state = AerowmState::new(&display_handle, engine);

    // Wayland listening socket (owned by us so its FD can be handed over
    // across an in-place restart) + client accept loop.
    match wayland_socket::setup_wayland_socket(event_loop.handle(), &display_handle) {
        Ok(info) => {
            state.wl_socket = Some(info);
        }
        Err(e) => {
            // Without a socket no Wayland client can ever connect; the
            // compositor is useless, so fail fast with a clear message.
            error!("Wayland socket setup failed: {e}");
            return Err(e);
        }
    }

    // In-place restart child: rebuild workspace scaffolding and queue
    // window placements for reconnecting clients.
    if std::env::var(session::RESTARTED_ENV).as_deref() == Ok("1") {
        match session::restore_session(&mut state) {
            Ok(n) => info!("restart handover complete ({n} windows to place)"),
            Err(e) => warn!("session restore failed ({e}); starting fresh"),
        }
    }

    // Initialize the IPC Unix Socket directly into calloop
    ipc::init_ipc_socket(event_loop.handle())?;

    // Initialize the backend: native DRM/KMS on TTY (or when requested),
    // nested winit window otherwise.
    let requested = std::env::args()
        .position(|a| a == "--backend")
        .and_then(|i| std::env::args().nth(i + 1))
        .or_else(|| std::env::var("AEROWM_BACKEND").ok());
    let use_udev = match requested.as_deref() {
        Some("udev") => true,
        Some("winit") => false,
        _ => {
            std::env::var("WAYLAND_DISPLAY").is_err() && std::env::var("DISPLAY").is_err()
        }
    };
    if use_udev {
        info!("Selecting native udev backend");
        if let Err(e) = backend::udev::init_udev(&mut event_loop, &mut display, &mut state) {
            warn!("Udev backend failed ({e}), falling back to nested winit");
            backend::winit::init_winit(&mut event_loop, &mut display, &mut state)?;
        }
    } else {
        backend::winit::init_winit(&mut event_loop, &mut display, &mut state)?;
    }

    // Legacy X11 support is opt-in (`--features xwayland`). When enabled,
    // boot the XWayland server; any failure (e.g. missing binary) only
    // disables X11 apps, never the Wayland session.
    #[cfg(feature = "xwayland")]
    if let Err(e) = state.init_xwayland(&display_handle, event_loop.handle()) {
        warn!("XWayland disabled: {e}");
    }

    info!("AeroWM initialization complete. Entering event loop.");

    // The main loop
    while state.is_running {
        event_loop.dispatch(None, &mut state)?;
        display.dispatch_clients(&mut state)?;
        display.flush_clients()?;
    }

    // Hot restart: dump the session, then re-exec over this process image
    // inheriting the Wayland listening socket.
    if state.restart_requested {
        match state.wl_socket {
            Some(ref info) => match session::dump_session(&state) {
                Ok(path) => {
                    info!(
                        "restarting on Wayland socket {} (fd={})",
                        info.name, info.fd
                    );
                    if let Err(e) = session::exec_restart(info.fd, &path) {
                        error!("hot restart failed: {e}");
                    }
                }
                Err(e) => error!("hot restart aborted, session dump failed: {e}"),
            },
            None => error!("hot restart aborted: no Wayland socket to hand over"),
        }
    }

    Ok(())
}
