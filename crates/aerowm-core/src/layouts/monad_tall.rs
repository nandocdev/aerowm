use crate::geometry::Rect;
use crate::layout::{Layout, split_evenly};

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
    fn name(&self) -> &'static str {
        "monad_tall"
    }

    fn apply(&self, area: Rect, num_windows: usize) -> Vec<Rect> {
        if num_windows == 0 {
            return vec![];
        }
        
        if num_windows <= self.master_count {
            let heights = split_evenly(area.size.height, num_windows);
            let mut rects = Vec::with_capacity(num_windows);
            let mut y = area.origin.y;
            for h in heights {
                rects.push(Rect::new(area.origin.x, y, area.size.width, h));
                y += h as i32;
            }
            return rects;
        }

        let master_width = (area.size.width as f32 * self.master_ratio) as u32;
        let stack_width = area.size.width - master_width;

        let master_heights = split_evenly(area.size.height, self.master_count);
        let stack_count = num_windows - self.master_count;
        let stack_heights = split_evenly(area.size.height, stack_count);

        let mut rects = Vec::with_capacity(num_windows);

        let mut y = area.origin.y;
        for h in master_heights {
            rects.push(Rect::new(area.origin.x, y, master_width, h));
            y += h as i32;
        }

        let mut y = area.origin.y;
        for h in stack_heights {
            rects.push(Rect::new(
                area.origin.x + master_width as i32,
                y,
                stack_width,
                h,
            ));
            y += h as i32;
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

    #[test]
    fn test_monad_tall_no_pixel_loss() {
        // 8 windows: 1 master + 7 stack; 1080 / 7 = 154 rem 2, so the
        // stack column must still tile exactly the full height.
        let layout = MonadTall::default();
        let area = Rect::new(0, 0, 1920, 1080);
        let rects = layout.apply(area, 8);
        assert_eq!(rects.len(), 8);
        let stack_sum: u32 = rects[1..].iter().map(|r| r.size.height).sum();
        assert_eq!(stack_sum, 1080, "stack column must cover full height");
        // Last window ends exactly at the area edge.
        let last = rects.last().unwrap();
        assert_eq!(last.origin.y + last.size.height as i32, 1080);
    }
}
