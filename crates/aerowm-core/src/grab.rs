//! Pure math for interactive move/resize grabs of floating windows.
//!
//! Everything here is plain data in, plain data out — no Smithay, no
//! compositor state. The corner clamping rules below are the most
//! bug-prone arithmetic in the window manager, so they live in a crate
//! that can be tested without a display server.

use crate::geometry::{Point, Rect, Size};

/// Minimum user-resizable width for floating windows.
pub const MIN_FLOAT_W: i32 = 120;
/// Minimum user-resizable height for floating windows.
pub const MIN_FLOAT_H: i32 = 80;

/// [`MIN_FLOAT_W`]/[`MIN_FLOAT_H`] as a [`Size`], the form
/// [`resize_from_corner`] takes.
pub const MIN_FLOAT_SIZE: Size = Size {
    width: MIN_FLOAT_W as u32,
    height: MIN_FLOAT_H as u32,
};

/// Corner grabbed for an interactive resize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrabCorner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Origin of a grabbed window after moving the pointer: the grab offset
/// is subtracted from the cursor so the window follows the pointer
/// without snapping its corner to it.
pub fn drag_origin(cursor_x: f64, cursor_y: f64, offset_x: f64, offset_y: f64) -> Point {
    Point {
        x: (cursor_x - offset_x).round() as i32,
        y: (cursor_y - offset_y).round() as i32,
    }
}

/// Integer pixel delta between two cursor samples.
pub fn cursor_delta(from_x: f64, from_y: f64, to_x: f64, to_y: f64) -> (i32, i32) {
    (
        (to_x - from_x).round() as i32,
        (to_y - from_y).round() as i32,
    )
}

/// Geometry of `start` after dragging `corner` by (`dx`, `dy`).
///
/// The opposite corner stays anchored and `min` bounds the result, so
/// dragging a corner past the opposite edge clamps at `min` instead of
/// inverting the rectangle. A window already smaller than `min` snaps
/// up to `min` on the first drag.
pub fn resize_from_corner(start: Rect, corner: GrabCorner, dx: i32, dy: i32, min: Size) -> Rect {
    let x1 = start.origin.x + start.size.width as i32;
    let y1 = start.origin.y + start.size.height as i32;
    let min_w = min.width as i32;
    let min_h = min.height as i32;
    let (nx, ny, nw, nh) = match corner {
        GrabCorner::TopLeft => {
            let nx = (start.origin.x + dx).min(x1 - min_w);
            let ny = (start.origin.y + dy).min(y1 - min_h);
            (nx, ny, x1 - nx, y1 - ny)
        }
        GrabCorner::TopRight => {
            let ny = (start.origin.y + dy).min(y1 - min_h);
            let nw = (start.size.width as i32 + dx).max(min_w);
            (start.origin.x, ny, nw, y1 - ny)
        }
        GrabCorner::BottomLeft => {
            let nx = (start.origin.x + dx).min(x1 - min_w);
            let nh = (start.size.height as i32 + dy).max(min_h);
            (nx, start.origin.y, x1 - nx, nh)
        }
        GrabCorner::BottomRight => {
            let nw = (start.size.width as i32 + dx).max(min_w);
            let nh = (start.size.height as i32 + dy).max(min_h);
            (start.origin.x, start.origin.y, nw, nh)
        }
    };
    Rect::new(nx, ny, nw as u32, nh as u32)
}

/// Corner of `rect` nearest to the cursor. Exact ties pick right/bottom.
pub fn nearest_corner(rect: &Rect, cursor_x: f64, cursor_y: f64) -> GrabCorner {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: u32, h: u32) -> Rect {
        Rect::new(x, y, w, h)
    }

    #[test]
    fn test_resize_grows_and_shrinks_from_bottom_right() {
        let start = rect(100, 200, 400, 300);
        let grown = resize_from_corner(start, GrabCorner::BottomRight, 50, -20, MIN_FLOAT_SIZE);
        assert_eq!(grown, rect(100, 200, 450, 280));

        let shrunk = resize_from_corner(start, GrabCorner::BottomRight, -100, -100, MIN_FLOAT_SIZE);
        assert_eq!(shrunk, rect(100, 200, 300, 200));
    }

    #[test]
    fn test_resize_from_bottom_right_clamps_at_minimum() {
        let start = rect(100, 200, 400, 300);
        let over = resize_from_corner(start, GrabCorner::BottomRight, -5000, -5000, MIN_FLOAT_SIZE);
        assert_eq!(over, rect(100, 200, 120, 80));
        assert!(over.size.width >= MIN_FLOAT_W as u32);
        assert!(over.size.height >= MIN_FLOAT_H as u32);
    }

    #[test]
    fn test_resize_from_top_left_moves_origin_and_keeps_far_corner() {
        let start = rect(100, 200, 400, 300);
        let moved = resize_from_corner(start, GrabCorner::TopLeft, 30, 40, MIN_FLOAT_SIZE);
        assert_eq!(moved, rect(130, 240, 370, 260));

        // The anchored corner (right/bottom edges) never moves.
        assert_eq!(
            moved.origin.x + moved.size.width as i32,
            start.origin.x + start.size.width as i32
        );
        assert_eq!(
            moved.origin.y + moved.size.height as i32,
            start.origin.y + start.size.height as i32
        );
    }

    #[test]
    fn test_resize_from_top_left_clamps_past_opposite_edge() {
        let start = rect(100, 200, 400, 300);
        // Dragging well past the anchored corner stops at the minimum
        // size instead of inverting the rectangle.
        let over = resize_from_corner(start, GrabCorner::TopLeft, 5000, 5000, MIN_FLOAT_SIZE);
        assert_eq!(over, rect(380, 420, 120, 80));
        assert_eq!(
            over,
            resize_from_corner(start, GrabCorner::TopLeft, 9000, 9000, MIN_FLOAT_SIZE)
        );
    }

    #[test]
    fn test_resize_from_mixed_corners_moves_one_axis_only() {
        let start = rect(100, 200, 400, 300);

        let top_right = resize_from_corner(start, GrabCorner::TopRight, 60, 25, MIN_FLOAT_SIZE);
        assert_eq!(top_right.origin.x, start.origin.x);
        assert_eq!(top_right.origin.y, 225);
        assert_eq!(top_right.size.width, 460);
        assert_eq!(top_right.size.height, 275);

        let bottom_left =
            resize_from_corner(start, GrabCorner::BottomLeft, -60, 25, MIN_FLOAT_SIZE);
        assert_eq!(bottom_left.origin.x, 40);
        assert_eq!(bottom_left.origin.y, start.origin.y);
        assert_eq!(bottom_left.size.width, 460);
        assert_eq!(bottom_left.size.height, 325);
    }

    #[test]
    fn test_resize_from_mixed_corners_clamps_each_axis_independently() {
        let start = rect(100, 200, 400, 300);
        // Horizontal drag collapses to the minimum width while the
        // vertical axis still follows the pointer.
        let top_right = resize_from_corner(start, GrabCorner::TopRight, -5000, 10, MIN_FLOAT_SIZE);
        assert_eq!(top_right, rect(100, 210, 120, 290));

        let bottom_left =
            resize_from_corner(start, GrabCorner::BottomLeft, 10, -5000, MIN_FLOAT_SIZE);
        assert_eq!(bottom_left, rect(110, 200, 390, 80));
    }

    #[test]
    fn test_resize_of_undersized_window_snaps_to_minimum() {
        let start = rect(100, 200, 50, 40);
        let dragged = resize_from_corner(start, GrabCorner::TopLeft, 0, 0, MIN_FLOAT_SIZE);
        assert_eq!(dragged, rect(30, 160, 120, 80));
    }

    #[test]
    fn test_resize_with_zero_delta_keeps_geometry() {
        let start = rect(37, -12, 640, 480);
        for corner in [
            GrabCorner::TopLeft,
            GrabCorner::TopRight,
            GrabCorner::BottomLeft,
            GrabCorner::BottomRight,
        ] {
            assert_eq!(
                resize_from_corner(start, corner, 0, 0, MIN_FLOAT_SIZE),
                start
            );
        }
    }

    #[test]
    fn test_nearest_corner_picks_quadrant() {
        let r = rect(0, 0, 800, 600);
        assert_eq!(nearest_corner(&r, 10.0, 10.0), GrabCorner::TopLeft);
        assert_eq!(nearest_corner(&r, 790.0, 10.0), GrabCorner::TopRight);
        assert_eq!(nearest_corner(&r, 10.0, 590.0), GrabCorner::BottomLeft);
        assert_eq!(nearest_corner(&r, 790.0, 590.0), GrabCorner::BottomRight);
        // Far outside the rect still resolves to the nearest corner.
        assert_eq!(nearest_corner(&r, -100.0, 5000.0), GrabCorner::BottomLeft);
        assert_eq!(nearest_corner(&r, 5000.0, 5000.0), GrabCorner::BottomRight);
    }

    #[test]
    fn test_nearest_corner_breaks_ties_right_and_bottom() {
        let r = rect(0, 0, 100, 100);
        // Dead center: equal distance to both edges on each axis.
        assert_eq!(nearest_corner(&r, 50.0, 50.0), GrabCorner::BottomRight);
    }

    #[test]
    fn test_drag_origin_keeps_grab_offset() {
        // Grabbed 10px right / 5px down from the origin: the origin lands
        // that far behind the cursor.
        let origin = drag_origin(510.4, 305.6, 10.0, 5.0);
        assert_eq!(origin, Point { x: 500, y: 301 });
    }

    #[test]
    fn test_drag_origin_rounds_to_nearest_pixel() {
        assert_eq!(drag_origin(10.4, 20.6, 0.0, 0.0), Point { x: 10, y: 21 });
        // Rust's `round` breaks halves away from zero on both axes.
        assert_eq!(drag_origin(-0.5, 0.5, 0.0, 0.0), Point { x: -1, y: 1 });
        assert_eq!(drag_origin(-10.5, 0.0, 0.0, 0.0), Point { x: -11, y: 0 });
    }

    #[test]
    fn test_cursor_delta_rounds_and_keeps_sign() {
        assert_eq!(cursor_delta(10.0, 20.0, 30.6, 5.2), (21, -15));
        assert_eq!(cursor_delta(0.0, 0.0, 0.0, 0.0), (0, 0));
    }
}
