use crate::geometry::Rect;
use crate::layout::Layout;

pub struct MonadTall {
    pub master_ratio: f32,
    pub master_count: usize,
}

impl Default for MonadTall {
    fn default() -> Self {
        Self {
            master_ratio: 0.5,
            master_count: 1,
        }
    }
}

impl Layout for MonadTall {
    fn apply(&self, area: Rect, num_windows: usize) -> Vec<Rect> {
        if num_windows == 0 {
            return vec![];
        }
        
        if num_windows <= self.master_count {
            let height_per_window = area.size.height / num_windows as u32;
            let mut rects = Vec::with_capacity(num_windows);
            for i in 0..num_windows {
                rects.push(Rect::new(
                    area.origin.x,
                    area.origin.y + (i as u32 * height_per_window) as i32,
                    area.size.width,
                    height_per_window,
                ));
            }
            return rects;
        }

        let master_width = (area.size.width as f32 * self.master_ratio) as u32;
        let stack_width = area.size.width - master_width;
        
        let master_height = area.size.height / self.master_count as u32;
        let stack_count = num_windows - self.master_count;
        let stack_height = area.size.height / stack_count as u32;

        let mut rects = Vec::with_capacity(num_windows);
        
        for i in 0..self.master_count {
            rects.push(Rect::new(
                area.origin.x,
                area.origin.y + (i as u32 * master_height) as i32,
                master_width,
                master_height,
            ));
        }
        
        for i in 0..stack_count {
            rects.push(Rect::new(
                area.origin.x + master_width as i32,
                area.origin.y + (i as u32 * stack_height) as i32,
                stack_width,
                stack_height,
            ));
        }

        rects
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_monad_tall_single_window() {
        let layout = MonadTall::default();
        let area = Rect::new(0, 0, 1920, 1080);
        let rects = layout.apply(area, 1);
        
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0], Rect::new(0, 0, 1920, 1080));
    }

    #[test]
    fn test_monad_tall_two_windows() {
        let layout = MonadTall::default();
        let area = Rect::new(0, 0, 1920, 1080);
        let rects = layout.apply(area, 2);
        
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0], Rect::new(0, 0, 960, 1080));
        assert_eq!(rects[1], Rect::new(960, 0, 960, 1080));
    }

    #[test]
    fn test_monad_tall_three_windows() {
        let layout = MonadTall::default();
        let area = Rect::new(0, 0, 1920, 1080);
        let rects = layout.apply(area, 3);
        
        assert_eq!(rects.len(), 3);
        assert_eq!(rects[0], Rect::new(0, 0, 960, 1080));
        assert_eq!(rects[1], Rect::new(960, 0, 960, 540));
        assert_eq!(rects[2], Rect::new(960, 540, 960, 540));
    }
}
