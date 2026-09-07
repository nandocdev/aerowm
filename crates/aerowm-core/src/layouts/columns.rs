use crate::geometry::Rect;
use crate::layout::{Layout, split_evenly};

pub struct Columns;

impl Layout for Columns {
    fn name(&self) -> &'static str {
        "columns"
    }

    fn apply(&self, area: Rect, num_windows: usize) -> Vec<Rect> {
        if num_windows == 0 {
            return vec![];
        }

        let widths = split_evenly(area.size.width, num_windows);
        let mut rects = Vec::with_capacity(num_windows);

        let mut x = area.origin.x;
        for w in widths {
            rects.push(Rect::new(x, area.origin.y, w, area.size.height));
            x += w as i32;
        }
        rects
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_columns_layout() {
        let layout = Columns;
        let area = Rect::new(0, 0, 1920, 1080);
        let rects = layout.apply(area, 3);
        
        assert_eq!(rects.len(), 3);
        assert_eq!(rects[0], Rect::new(0, 0, 640, 1080));
        assert_eq!(rects[1], Rect::new(640, 0, 640, 1080));
        assert_eq!(rects[2], Rect::new(1280, 0, 640, 1080));
    }

    #[test]
    fn test_columns_no_pixel_loss() {
        // 1920 / 7 = 274 rem 2: widths must still sum to the full width.
        let layout = Columns;
        let area = Rect::new(0, 0, 1920, 1080);
        let rects = layout.apply(area, 7);
        assert_eq!(rects.len(), 7);
        let total: u32 = rects.iter().map(|r| r.size.width).sum();
        assert_eq!(total, 1920, "columns must cover full width");
        let last = rects.last().unwrap();
        assert_eq!(last.origin.x + last.size.width as i32, 1920);
    }
}
