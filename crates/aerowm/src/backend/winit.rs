use calloop::{timer::{Timer, TimeoutAction}, EventLoop};
use smithay::backend::input::InputEvent;
use smithay::backend::winit::{self, WinitEvent};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::output::{Output, PhysicalProperties};
use smithay::utils::{Point, Size, Transform, Physical};
use wayland_server::Display;
use crate::state::AerowmState;
use crate::input as input_dispatch;
use tracing::{info, warn};
use std::time::Duration;

pub fn init_winit(
    event_loop: &mut EventLoop<AerowmState>,
    display: &mut Display<AerowmState>,
    state: &mut AerowmState,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Initializing Winit Backend (Nested Wayland)");
    
    // Initialize the Smithay winit backend specifying the GlesRenderer for hardware acceleration
    let (backend, mut winit) = winit::init::<GlesRenderer>().map_err(|e| e.to_string())?;
    
    // Create output
    let output = Output::new(
        "winit".to_string(),
        PhysicalProperties {
            size: Size::from((0, 0)),
            subpixel: smithay::output::Subpixel::Unknown,
            make: "Smithay".to_string(),
            model: "Winit".to_string(),
        },
    );
    
    // Set up output mode (will be updated on resize)
    let mode = smithay::output::Mode {
        size: Size::from((1920, 1080)),
        refresh: 60000,
    };
    output.add_mode(mode);
    output.set_preferred(mode);
    
    // Create global for the output
    output.create_global::<AerowmState>(&display.handle());
    
    // Initialize damage tracker from output
    let damage_tracker = OutputDamageTracker::from_output(&output);
    
    // Store in state
    state.backend = Some(backend);
    state.output = Some(output.clone());
    state.damage_tracker = Some(damage_tracker);
    state.output_size = Some(mode.size);
    
    // Map output in space at (0, 0)
    state.space.map_output(&output, Point::from((0, 0)));
    
    // Apply initial layout
    state.apply_layout();
    
    info!("Winit backend initialized with output: {}", output.name());
    
    // Drive the winit event loop at ~60fps using Calloop's timer source
    let timer = Timer::immediate();
    event_loop.handle().insert_source(timer, move |_, _, state| {
        winit.dispatch_new_events(|event| {
            match event {
                WinitEvent::Resized { size, .. } => {
                    info!("Window resized to {:?}", size);
                    handle_resize(state, size);
                }
                WinitEvent::CloseRequested => {
                    info!("Window closed. Exiting.");
                    state.is_running = false;
                }
                WinitEvent::Redraw => {
                    render_frame(state);
                }
                WinitEvent::Input(event) => match event {
                    InputEvent::Keyboard { event } => {
                        input_dispatch::handle_keyboard(state, event);
                    }
                    InputEvent::PointerMotionAbsolute { event } => {
                        input_dispatch::handle_pointer_motion_absolute(state, event);
                    }
                    InputEvent::PointerButton { event } => {
                        input_dispatch::handle_pointer_button(state, event);
                    }
                    InputEvent::PointerAxis { event } => {
                        input_dispatch::handle_pointer_axis(state, event);
                    }
                    _ => {}
                },
                _ => {}
            }
        });

        // Continue running the timer every 16ms (~60 FPS)
        TimeoutAction::ToDuration(Duration::from_millis(16))
    }).map_err(|e| format!("Timer error: {}", e))?;
    
    Ok(())
}

fn handle_resize(state: &mut AerowmState, size: Size<i32, Physical>) {
    if let Some(output) = &state.output {
        let mode = smithay::output::Mode {
            size,
            refresh: 60000,
        };
        output.add_mode(mode);
        output.set_preferred(mode);
        output.change_current_state(Some(mode), None, None, None);
    }
    
    if let Some(damage_tracker) = &mut state.damage_tracker {
        *damage_tracker = OutputDamageTracker::new(size, 1.0, Transform::Normal);
    }
    
    state.output_size = Some(size);
    if let Some(output) = &state.output {
        state.space.map_output(output, Point::from((0, 0)));
    }
    state.apply_layout();
}

fn render_frame(state: &mut AerowmState) {
    // Clean up dead windows first (before borrowing backend/output)
    state.cleanup_dead_windows();
    
    let Some(backend) = &mut state.backend else { return };
    let Some(output) = &state.output else { return };
    let Some(damage_tracker) = &mut state.damage_tracker else { return };
    
    // Get buffer age for damage tracking before binding
    let age = backend.buffer_age().unwrap_or(0);
    
    // Get render elements from space (need renderer for this)
    let (renderer, mut framebuffer) = match backend.bind() {
        Ok(fb) => fb,
        Err(e) => {
            warn!("Failed to bind backend: {}", e);
            return;
        }
    };
    
    let mut lock_elements = Vec::new();
    if state.is_locked {
        if let Some(lock_surface) = state.lock_surfaces.first() {
            use smithay::backend::renderer::element::surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement};
            use smithay::backend::renderer::element::Kind;
            let mut tree = render_elements_from_surface_tree::<_, WaylandSurfaceRenderElement<GlesRenderer>>(
                renderer,
                lock_surface.wl_surface(),
                (0, 0),
                1.0,
                1.0,
                Kind::Unspecified,
            );
            lock_elements.append(&mut tree);
        }
    }

    let elements = if !state.is_locked {
        match state.space.render_elements_for_output(renderer, output, 1.0) {
            Ok(e) => e,
            Err(e) => {
                warn!("Failed to get render elements: {}", e);
                return;
            }
        }
    } else {
        Vec::new() // We can't use SpaceRenderElements when locked unless we define a macro.
    };
    
    // Render with damage tracking
    let clear_color = [0.0, 0.0, 0.0, 1.0];
    let render_result = if state.is_locked {
        damage_tracker.render_output(
            renderer,
            &mut framebuffer,
            age,
            &lock_elements,
            clear_color,
        )
    } else {
        damage_tracker.render_output(
            renderer,
            &mut framebuffer,
            age,
            &elements,
            clear_color,
        )
    };
    
    // Extract damage before dropping framebuffer
    let damage = render_result.as_ref().ok().and_then(|r| r.damage).map(|v| v.clone());
    
    // Drop framebuffer and renderer
    drop(framebuffer);
    let _ = renderer;
    
    match render_result {
        Ok(_) => {
            // Submit the frame with damage
            if let Err(e) = backend.submit(damage.as_deref()) {
                warn!("Failed to submit frame: {}", e);
            }
        }
        Err(e) => {
            warn!("Failed to render output: {}", e);
        }
    }
    
    // Send frame callbacks
    let now = Duration::from_millis(0);
    state.space.elements().for_each(|window| {
        window.send_frame(output, now, None, |_, _| Some(output.clone()));
    });
    
    state.space.refresh();
    output.cleanup();
}