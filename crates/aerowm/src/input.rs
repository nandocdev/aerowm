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

pub fn handle_keyboard(state: &mut AerowmState, event: WinitKeyboardInputEvent) {
    let serial = SERIAL_COUNTER.next_serial();
    let time = event.time() as u32;
    let keycode = event.key_code();
    let key_state = event.state();

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

fn pointer_focus_under(
    state: &AerowmState,
    pos: Point<f64, Logical>,
) -> Option<(WlSurface, Point<f64, Logical>)> {
    let (window, _) = state.space.element_under(pos)?;
    let surface = window.toplevel()?.wl_surface().clone();
    Some((surface, pos))
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
    state.pointer_location = pos;

    let serial = SERIAL_COUNTER.next_serial();
    let time = event.time() as u32;
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

pub fn handle_pointer_button(state: &mut AerowmState, event: WinitMouseInputEvent) {
    let serial = SERIAL_COUNTER.next_serial();
    let time = event.time() as u32;

    // Click-to-focus: focus the window under the cursor on press.
    if event.state() == ButtonState::Pressed {
        let pos = state.pointer_location;
        let toplevel = state
            .space
            .element_under(pos)
            .and_then(|(w, _)| w.toplevel().cloned());
        if let Some(toplevel) = toplevel {
            state.focus_toplevel(&toplevel);
        }
    }

    let Some(pointer) = state.seat.get_pointer() else {
        return;
    };
    pointer.button(
        state,
        &ButtonEvent {
            button: event.button_code(),
            state: event.state(),
            serial,
            time,
        },
    );
}

pub fn handle_pointer_axis(state: &mut AerowmState, event: WinitMouseWheelEvent) {
    // Scroll is forwarded implicitly via axis frames in a full impl;
    // for sprint 4 we just log it so seat/pointer wiring stays verifiable.
    debug!("pointer axis event (ignored for now): {event:?}");
    let _ = state;
}
