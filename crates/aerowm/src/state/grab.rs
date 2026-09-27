//! Interactive pointer grabs: dragging a floating window around and
//! resizing it from a corner.

use aerowm_core::geometry::Rect as CoreRect;
use aerowm_core::id::WindowId;
use smithay::utils::{Logical, Point};

use super::{AerowmState, MIN_FLOAT_H, MIN_FLOAT_W};

/// Corner grabbed for an interactive resize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrabCorner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

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
                let nx = (cursor_x - dx).round() as i32;
                let ny = (cursor_y - dy).round() as i32;
                if let Some(window) = self.window_object(id) {
                    self.space.map_element(window, Point::from((nx, ny)), false);
                }
                if let Some(geo) = self.float_geo.get_mut(&id) {
                    geo.origin.x = nx;
                    geo.origin.y = ny;
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
                let dx = (cursor_x - sx).round() as i32;
                let dy = (cursor_y - sy).round() as i32;
                let x1 = start.origin.x + start.size.width as i32;
                let y1 = start.origin.y + start.size.height as i32;
                let (nx, ny, nw, nh) = match corner {
                    GrabCorner::TopLeft => {
                        let nx = (start.origin.x + dx).min(x1 - MIN_FLOAT_W);
                        let ny = (start.origin.y + dy).min(y1 - MIN_FLOAT_H);
                        (nx, ny, x1 - nx, y1 - ny)
                    }
                    GrabCorner::TopRight => {
                        let ny = (start.origin.y + dy).min(y1 - MIN_FLOAT_H);
                        let nw = (start.size.width as i32 + dx).max(MIN_FLOAT_W);
                        (start.origin.x, ny, nw, y1 - ny)
                    }
                    GrabCorner::BottomLeft => {
                        let nx = (start.origin.x + dx).min(x1 - MIN_FLOAT_W);
                        let nh = (start.size.height as i32 + dy).max(MIN_FLOAT_H);
                        (nx, start.origin.y, x1 - nx, nh)
                    }
                    GrabCorner::BottomRight => {
                        let nw = (start.size.width as i32 + dx).max(MIN_FLOAT_W);
                        let nh = (start.size.height as i32 + dy).max(MIN_FLOAT_H);
                        (start.origin.x, start.origin.y, nw, nh)
                    }
                };
                if let Some(window) = self.window_object(id) {
                    self.space.map_element(window, Point::from((nx, ny)), false);
                }
                let resized = CoreRect::new(nx, ny, nw as u32, nh as u32);
                if let Some(surface) = self.surfaces.get(&id) {
                    surface.with_pending_state(|s| {
                        s.size = Some((nw, nh).into());
                    });
                    surface.send_configure();
                }
                #[cfg(feature = "xwayland")]
                if !self.surfaces.contains_key(&id)
                    && let Some(x11) = self.x11_surfaces.get(&id)
                {
                    let geo = smithay::utils::Rectangle::new((nx, ny).into(), (nw, nh).into());
                    if let Err(e) = x11.configure(Some(geo)) {
                        tracing::warn!("failed to configure X11 window: {e:?}");
                    }
                }
                self.float_geo.insert(id, resized);
            }
        }
    }

    pub fn end_grab(&mut self) {
        self.grab = PointerGrabState::None;
    }
}

/// Corner of a rectangle nearest to the cursor position.
fn nearest_corner(rect: &CoreRect, cursor_x: f64, cursor_y: f64) -> GrabCorner {
    let x0 = rect.origin.x as f64;
    let y0 = rect.origin.y as f64;
    let x1 = x0 + rect.size.width as f64;
    let y1 = y0 + rect.size.height as f64;
    let left = (cursor_x - x0).abs() < (cursor_x - x1).abs();
    let top = (cursor_y - y0).abs() < (cursor_y - y1).abs();
    match (top, left) {
        (true, true) => GrabCorner::TopLeft,
        (true, false) => GrabCorner::TopRight,
        (false, true) => GrabCorner::BottomLeft,
        (false, false) => GrabCorner::BottomRight,
    }
}
