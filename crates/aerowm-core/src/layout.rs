use crate::geometry::Rect;

/// A layout algorithm takes an available bounding rectangle
/// and the number of windows to arrange, and returns a 
/// vector of rectangles representing the geometry for each window.
pub trait Layout {
    fn apply(&self, area: Rect, num_windows: usize) -> Vec<Rect>;
}
