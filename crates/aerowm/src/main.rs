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

    let mut event_loop: EventLoop<AerowmState> = EventLoop::try_new()?;
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

    // Initialize the backend
    backend::winit::init_winit(&mut event_loop, &mut display, &mut state)?;

    info!("AeroWM initialization complete. Entering event loop.");

    // The main loop
    while state.is_running {
        event_loop.dispatch(None, &mut state)?;
        display.flush_clients()?;
    }

    Ok(())
}
