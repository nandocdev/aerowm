use calloop::{timer::{Timer, TimeoutAction}, EventLoop};
use smithay::backend::winit::{self, WinitEvent};
use smithay::backend::renderer::gles::GlesRenderer;
use wayland_server::Display;
use crate::state::AerowmState;
use tracing::info;
use std::time::Duration;

pub fn init_winit(
    event_loop: &mut EventLoop<AerowmState>,
    _display: &mut Display<AerowmState>,
    _state: &mut AerowmState,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Initializing Winit Backend (Nested Wayland)");
    
    // Initialize the Smithay winit backend specifying the GlesRenderer for hardware acceleration
    let (mut _backend, mut winit) = winit::init::<GlesRenderer>().map_err(|e| e.to_string())?;

    // Drive the winit event loop at ~60fps using Calloop's timer source
    let timer = Timer::immediate();
    event_loop.handle().insert_source(timer, move |_, _, state| {
        winit.dispatch_new_events(|event| {
            match event {
                WinitEvent::Resized { size, .. } => {
                    info!("Window resized to {:?}", size);
                }
                WinitEvent::CloseRequested => {
                    info!("Window closed. Exiting.");
                    state.is_running = false;
                }
                _ => {}
            }
        });

        // Continue running the timer every 16ms (~60 FPS)
        TimeoutAction::ToDuration(Duration::from_millis(16))
    }).map_err(|e| format!("Timer error: {}", e))?;

    Ok(())
}
