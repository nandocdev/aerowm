use smithay::backend::input::{
    AbsolutePositionEvent, ButtonState, Event, KeyState, KeyboardKeyEvent, PointerButtonEvent,
};
use smithay::backend::winit::{
    WinitKeyboardInputEvent, WinitMouseInputEvent, WinitMouseMovedEvent, WinitMouseWheelEvent,
};
use smithay::input::keyboard::{FilterResult, ModifiersState};
use smithay::input::pointer::{ButtonEvent, MotionEvent};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, SERIAL_COUNTER};
use tracing::{debug, info};

use crate::state::AerowmState;

/// Builds the canonical combo string, e.g. `"Super+Return"`, `"Super+Shift+q"`.
/// Must match the Luau `aerowm.mods` values: Super / Alt / Control / Shift.
pub fn build_combo(mods: &ModifiersState, keysym_name: &str) -> String {
    let mut parts = Vec::with_capacity(5);
    if mods.logo {
        parts.push("Super");
    }
    if mods.alt {
        parts.push("Alt");
    }
    if mods.ctrl {
        parts.push("Control");
    }
    if mods.shift {
        parts.push("Shift");
    }
    // Normalize single letters to lowercase so `Shift+q` doesn't become
    // `Shift+Q` (xkb returns the shifted symbol via modified_sym).
    let mut key = keysym_name.to_string();
    if key.len() == 1 {
        key = key.to_lowercase();
    }
    parts.push(&key);
    parts.join("+")
}

/// Tries the user Luau bind first, then the built-in fallback actions.
/// Returns `true` if the combo was handled (→ Intercept, don't forward).
pub fn dispatch_keybinding(state: &mut AerowmState, combo: &str) -> bool {
    // 1. User config wins.
    if state.engine.trigger_bind(combo) {
        info!("keybind dispatched to Luau: {combo}");
        return true;
    }

    // 2. Built-in fallback so the WM is usable with an empty config.
    match combo {
        "Super+q" => {
            info!("keybind: kill_active");
            state.kill_active();
            true
        }
        "Super+j" | "Super+Tab" => {
            state.focus_next();
            true
        }
        "Super+k" => {
            state.focus_prev();
            true
        }
        "Super+Return" => {
            info!("keybind: spawn terminal");
            state.spawn("kitty");
            true
        }
        "Super+m" => {
            state.swap_master();
            true
        }
        "Super+h" | "Super+Left" | "Super+comma" => {
            state.prev_workspace();
            true
        }
        "Super+l" | "Super+Right" | "Super+period" => {
            state.next_workspace();
            true
        }
        "Super+Shift+q" => {
            info!("keybind: exit");
            state.is_running = false;
            true
        }
        _ => {
            // Super+1..9 → direct workspace switch.
            if let Some(digit) = combo.strip_prefix("Super+")
                && digit.len() == 1
                && let Some(n) = digit.chars().next().and_then(|c| c.to_digit(10))
                && (1..=9).contains(&n)
            {
                state.switch_workspace((n - 1) as usize);
                return true;
            }
            false
        }
    }
}

/// Backend-agnostic keyboard dispatch shared by winit and libinput.
/// Feeds the key through the seat's xkb state; global binds intercept.
pub fn handle_keyboard_input(
    state: &mut AerowmState,
    keycode: smithay::backend::input::Keycode,
    key_state: KeyState,
    time: u32,
) {
    let serial = SERIAL_COUNTER.next_serial();

    let Some(keyboard) = state.seat.get_keyboard() else {
        return;
    };

    keyboard.input(
        state,
        keycode,
        key_state,
        serial,
        time,
        |state, mods, keysym| {
            if key_state == KeyState::Pressed {
                let name = xkbcommon::xkb::keysym_get_name(keysym.modified_sym());
                let combo = build_combo(mods, &name);
                debug!("key pressed: {combo}");
                if dispatch_keybinding(state, &combo) {
                    return FilterResult::Intercept(());
                }
            }
            FilterResult::Forward
        },
    );
}

pub fn handle_keyboard(state: &mut AerowmState, event: WinitKeyboardInputEvent) {
    handle_keyboard_input(state, event.key_code(), event.state(), event.time() as u32);
}

/// Topmost layer-shell surface under the cursor, if any.
///
/// Layers (bars, launchers, notifications) sit above tiled windows, so they
/// are hit-tested first. Returns the surface; rank prefers Overlay > Top >
/// Bottom > Background, then most-recently mapped.
fn layer_focus_under(state: &AerowmState, pos: Point<f64, Logical>) -> Option<WlSurface> {
    use smithay::desktop::layer_map_for_output;
    use smithay::wayland::shell::wlr_layer::Layer;

    fn rank(layer: Layer) -> u8 {
        match layer {
            Layer::Background => 0,
            Layer::Bottom => 1,
            Layer::Top => 2,
            Layer::Overlay => 3,
        }
    }

    let mut best: Option<(u8, usize, WlSurface)> = None;
    for output in state.space.outputs() {
        let origin = state
            .space
            .output_geometry(output)
            .map(|g| g.loc)
            .unwrap_or_default();
        let map = layer_map_for_output(output);
        for (idx, layer) in map.layers().enumerate() {
            let Some(geo) = map.layer_geometry(layer) else {
                continue;
            };
            let rect = smithay::utils::Rectangle::new(
                (origin.x + geo.loc.x, origin.y + geo.loc.y).into(),
                (geo.size.w, geo.size.h).into(),
            );
            if rect.contains(pos.to_i32_round()) {
                let candidate = (rank(layer.layer()), idx, layer.wl_surface().clone());
                if best.as_ref().map(|b| (b.0, b.1) <= (candidate.0, candidate.1)).unwrap_or(true) {
                    best = Some(candidate);
                }
            }
        }
    }
    best.map(|(_, _, surface)| surface)
}

fn pointer_focus_under(
    state: &AerowmState,
    pos: Point<f64, Logical>,
) -> Option<(WlSurface, Point<f64, Logical>)> {
    // Layers first so bars and launchers receive pointer input.
    if let Some(surface) = layer_focus_under(state, pos) {
        return Some((surface, pos));
    }
    let (window, _) = state.space.element_under(pos)?;
    let surface = window.toplevel()?.wl_surface().clone();
    Some((surface, pos))
}

/// `true` when a layer-shell surface covers the cursor position.
fn layer_under_cursor(state: &AerowmState) -> bool {
    layer_focus_under(state, state.pointer_location).is_some()
}

/// Moves the pointer to an absolute logical position (shared path).
pub fn pointer_motion_to(state: &mut AerowmState, pos: Point<f64, Logical>, time: u32) {
    state.pointer_location = pos;

    let serial = SERIAL_COUNTER.next_serial();
    let Some(pointer) = state.seat.get_pointer() else {
        return;
    };

    let focus = pointer_focus_under(state, pos);
    pointer.motion(
        state,
        focus,
        &MotionEvent {
            location: pos,
            serial,
            time,
        },
    );
}

/// Applies a relative pointer delta, clamped to the primary output bounds.
pub fn pointer_motion_relative(state: &mut AerowmState, delta: Point<f64, Logical>, time: u32) {
    let mut pos = state.pointer_location + delta;
    // Clamp into the primary output so the cursor can't get lost.
    if let Some(size) = state.output_size {
        pos.x = pos.x.clamp(0.0, size.w as f64 - 1.0);
        pos.y = pos.y.clamp(0.0, size.h as f64 - 1.0);
    } else if let Some(geometry) = state.space.outputs().next().and_then(|o| state.space.output_geometry(o)) {
        pos.x = pos.x.clamp(
            geometry.loc.x as f64,
            (geometry.loc.x + geometry.size.w) as f64 - 1.0,
        );
        pos.y = pos.y.clamp(
            geometry.loc.y as f64,
            (geometry.loc.y + geometry.size.h) as f64 - 1.0,
        );
    }
    pointer_motion_to(state, pos, time);
}

/// Shared button path: click-to-focus on press, then forward to the client.
pub fn pointer_button_event(
    state: &mut AerowmState,
    button: u32,
    btn_state: ButtonState,
    time: u32,
) {
    // Click-to-focus: focus the window under the cursor on press, unless
    // a layer-shell surface (bar, launcher) is on top — layers keep their
    // own input without stealing tiling focus.
    if btn_state == ButtonState::Pressed && !layer_under_cursor(state) {
        let pos = state.pointer_location;
        let toplevel = state
            .space
            .element_under(pos)
            .and_then(|(w, _)| w.toplevel().cloned());
        if let Some(toplevel) = toplevel {
            state.focus_toplevel(&toplevel);
        }
    }

    let serial = SERIAL_COUNTER.next_serial();
    let Some(pointer) = state.seat.get_pointer() else {
        return;
    };
    pointer.button(
        state,
        &ButtonEvent {
            button,
            state: btn_state,
            serial,
            time,
        },
    );
}

pub fn handle_pointer_motion_absolute(
    state: &mut AerowmState,
    event: WinitMouseMovedEvent,
) {
    let Some(output_size) = state.output_size else {
        return;
    };
    let pos = Point::from((
        event.x_transformed(output_size.w),
        event.y_transformed(output_size.h),
    ));
    pointer_motion_to(state, pos, event.time() as u32);
}

pub fn handle_pointer_button(state: &mut AerowmState, event: WinitMouseInputEvent) {
    pointer_button_event(
        state,
        event.button_code(),
        event.state(),
        event.time() as u32,
    );
}

pub fn handle_pointer_axis(state: &mut AerowmState, event: WinitMouseWheelEvent) {
    // Scroll is forwarded implicitly via axis frames in a full impl;
    // for sprint 4 we just log it so seat/pointer wiring stays verifiable.
    debug!("pointer axis event (ignored for now): {event:?}");
    let _ = state;
}

/// Routes libinput (udev backend) events into the shared dispatch paths.
pub fn handle_libinput_event(
    state: &mut AerowmState,
    event: smithay::backend::input::InputEvent<smithay::backend::libinput::LibinputInputBackend>,
) {
    use smithay::backend::input::InputEvent;
    use smithay::backend::input::{
        KeyboardKeyEvent as _, PointerButtonEvent as _, PointerMotionEvent as _,
    };

    match event {
        InputEvent::Keyboard { event } => {
            handle_keyboard_input(state, event.key_code(), event.state(), event.time() as u32);
        }
        InputEvent::PointerMotion { event } => {
            let time = event.time() as u32;
            pointer_motion_relative(state, event.delta(), time);
        }
        InputEvent::PointerMotionAbsolute { event } => {
            use smithay::backend::input::AbsolutePositionEvent as _;
            // Libinput absolute coords are normalized [0,1]; scale by primary output.
            let (w, h) = state
                .output_size
                .map(|s| (s.w as f64, s.h as f64))
                .or_else(|| {
                    state
                        .space
                        .outputs()
                        .next()
                        .and_then(|o| state.space.output_geometry(o))
                        .map(|g| (g.size.w as f64, g.size.h as f64))
                })
                .unwrap_or((1920.0, 1080.0));
            let time = event.time() as u32;
            pointer_motion_to(
                state,
                Point::from((event.x() * w, event.y() * h)),
                time,
            );
        }
        InputEvent::PointerButton { event } => {
            pointer_button_event(
                state,
                event.button_code(),
                event.state(),
                event.time() as u32,
            );
        }
        InputEvent::PointerAxis { event } => {
            debug!("libinput axis event (ignored for now): {event:?}");
        }
        _ => {}
    }
}
