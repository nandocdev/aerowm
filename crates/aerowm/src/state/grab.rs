//! Interactive pointer grabs: dragging a floating window around and
//! resizing it from a corner.
//!
//! The arithmetic lives in [`aerowm_core::grab`]; this module owns the
//! grab lifecycle — starting one, advancing it with pointer motion and
//! applying the resulting geometry, whatever backend the window is on.

use aerowm_core::geometry::Rect as CoreRect;
use aerowm_core::grab::{
    GrabCorner, MIN_FLOAT_SIZE, cursor_delta, drag_origin, nearest_corner, resize_from_corner,
};
use aerowm_core::id::WindowId;
use smithay::utils::{Logical, Point};

use super::AerowmState;

/// Pointer grab state for interactive move/resize of floating windows.
///
/// While a grab is active, pointer motion and the grabbing button are
/// consumed by the compositor (not forwarded to clients).
#[derive(Debug, Clone, Copy)]
pub enum PointerGrabState {
    None,
    Move {
        id: WindowId,
        button: u32,
        dx: f64,
        dy: f64,
    },
    Resize {
        id: WindowId,
        button: u32,
        corner: GrabCorner,
        start: CoreRect,
        cursor_x: f64,
        cursor_y: f64,
    },
}

impl PointerGrabState {
    pub fn is_active(&self) -> bool {
        !matches!(self, PointerGrabState::None)
    }

    /// Button that initiated the grab, if any.
    pub fn button(&self) -> Option<u32> {
        match *self {
            PointerGrabState::None => None,
            PointerGrabState::Move { button, .. } | PointerGrabState::Resize { button, .. } => {
                Some(button)
            }
        }
    }

    /// Window being manipulated, if any.
    pub fn window_id(&self) -> Option<WindowId> {
        match *self {
            PointerGrabState::None => None,
            PointerGrabState::Move { id, .. } | PointerGrabState::Resize { id, .. } => Some(id),
        }
    }
}

impl AerowmState {
    /// Starts an interactive move. The window is floated first (pinning its
    /// geometry) and the rest of the workspace re-tiles around the gap.
    pub fn begin_move_grab(&mut self, id: WindowId, button: u32, cursor: Point<f64, Logical>) {
        if self.float_window(id) {
            self.apply_layout();
        }
        let Some(pinned) = self.float_geo.get(&id).copied() else {
            self.grab = PointerGrabState::None;
            return;
        };
        self.active_workspace_mut().focus_window(id);
        self.update_keyboard_focus();
        self.grab = PointerGrabState::Move {
            id,
            button,
            dx: cursor.x - pinned.origin.x as f64,
            dy: cursor.y - pinned.origin.y as f64,
        };
    }

    /// Starts an interactive resize from the corner nearest to the cursor.
    pub fn begin_resize_grab(&mut self, id: WindowId, button: u32, cursor: Point<f64, Logical>) {
        if self.float_window(id) {
            self.apply_layout();
        }
        let Some(pinned) = self.float_geo.get(&id).copied() else {
            self.grab = PointerGrabState::None;
            return;
        };
        let corner = nearest_corner(&pinned, cursor.x, cursor.y);
        self.active_workspace_mut().focus_window(id);
        self.update_keyboard_focus();
        self.grab = PointerGrabState::Resize {
            id,
            button,
            corner,
            start: pinned,
            cursor_x: cursor.x,
            cursor_y: cursor.y,
        };
    }

    /// Advances the active grab to the cursor position. No-op without a grab.
    pub fn update_grab_to(&mut self, cursor_x: f64, cursor_y: f64) {
        match self.grab {
            PointerGrabState::None => {}
            PointerGrabState::Move { id, dx, dy, .. } => {
                let origin = drag_origin(cursor_x, cursor_y, dx, dy);
                if let Some(window) = self.window_object(id) {
                    self.space
                        .map_element(window, Point::from((origin.x, origin.y)), false);
                }
                if let Some(geo) = self.float_geo.get_mut(&id) {
                    geo.origin = origin;
                }
            }
            PointerGrabState::Resize {
                id,
                corner,
                start,
                cursor_x: sx,
                cursor_y: sy,
                ..
            } => {
                let (dx, dy) = cursor_delta(sx, sy, cursor_x, cursor_y);
                let resized = resize_from_corner(start, corner, dx, dy, MIN_FLOAT_SIZE);
                if let Some(window) = self.window_object(id) {
                    self.space.map_element(
                        window,
                        Point::from((resized.origin.x, resized.origin.y)),
                        false,
                    );
                }
                if let Some(surface) = self.configurable(id) {
                    surface.push_geometry(resized);
                }
                self.float_geo.insert(id, resized);
            }
        }
    }

    pub fn end_grab(&mut self) {
        self.grab = PointerGrabState::None;
    }
}
