use crate::geometry::Rect;
use crate::layout::Layout;

pub struct Columns;

impl Layout for Columns {
    fn apply(&self, area: Rect, num_windows: usize) -> Vec<Rect> {
        if num_windows == 0 {
            return vec![];
        }
        
        let width_per_window = area.size.width / num_windows as u32;
        let mut rects = Vec::with_capacity(num_windows);
        
        for i in 0..num_windows {
            rects.push(Rect::new(
                area.origin.x + (i as u32 * width_per_window) as i32,
                area.origin.y,
                width_per_window,
                area.size.height,
            ));
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
}
