mod state;
mod backend;
mod input;
mod ipc;
mod handlers;

use tracing::{info, warn};
use calloop::EventLoop;
use wayland_server::Display;
use aerowm_lua::ScriptEngine;
use crate::state::AerowmState;

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

    info!("AeroWM initialization complete. Entering event loop.");

    // The main loop
    while state.is_running {
        event_loop.dispatch(None, &mut state)?;
        display.flush_clients()?;
    }

    Ok(())
}
