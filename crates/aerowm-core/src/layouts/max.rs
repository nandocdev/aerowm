use crate::geometry::Rect;
use crate::layout::Layout;

pub struct Max;

impl Layout for Max {
    fn apply(&self, area: Rect, num_windows: usize) -> Vec<Rect> {
        vec![area; num_windows]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_max_layout() {
        let layout = Max;
        let area = Rect::new(10, 10, 1000, 500);
        let rects = layout.apply(area, 5);
        
        assert_eq!(rects.len(), 5);
        for rect in rects {
            assert_eq!(rect, area);
        }
    }
}
