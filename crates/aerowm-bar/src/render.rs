//! Software rendering of the bar into an ARGB8888 SHM canvas.
//!
//! Text uses embedded bitmap fonts (no system font dependencies).

use std::convert::Infallible;

use embedded_graphics::{
    mono_font::{MonoTextStyle, ascii::FONT_6X13},
    pixelcolor::Rgb888,
    prelude::*,
    text::{Baseline, Text},
};

use aerowm_ipc::CompositorSnapshot;

use crate::config::{BarConfig, Widget};

/// Clickable workspace cell, in surface-local coordinates.
#[derive(Debug, Clone, Copy)]
pub struct WorkspaceCell {
    pub x0: u32,
    pub x1: u32,
    pub index: usize,
}

/// ARGB pixel buffer view.
pub struct Canvas<'a> {
    buf: &'a mut [u8],
    pub width: u32,
    pub height: u32,
}

impl<'a> Canvas<'a> {
    pub fn new(buf: &'a mut [u8], width: u32, height: u32) -> Self {
        Self { buf, width, height }
    }

    pub fn fill(&mut self, color: u32) {
        let bytes = color.to_le_bytes();
        let (chunks, _) = self.buf.as_chunks_mut::<4>();
        for chunk in chunks {
            chunk.copy_from_slice(&bytes);
        }
    }

    pub fn fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, color: u32) {
        let bytes = color.to_le_bytes();
        let x1 = (x + w).min(self.width);
        let y1 = (y + h).min(self.height);
        for row in y.min(self.height)..y1 {
            let base = (row * self.width + x.min(self.width)) as usize * 4;
            let end = (row * self.width + x1) as usize * 4;
            if base < end && end <= self.buf.len() {
                let (chunks, _) = self.buf[base..end].as_chunks_mut::<4>();
                for chunk in chunks {
                    chunk.copy_from_slice(&bytes);
                }
            }
        }
    }

    fn put_rgb(&mut self, x: u32, y: u32, color: Rgb888) {
        if x >= self.width || y >= self.height {
            return;
        }
        let off = (y * self.width + x) as usize * 4;
        if off + 4 <= self.buf.len() {
            // Opaque overlay: embedded fonts have no alpha blending here.
            self.buf[off] = color.b();
            self.buf[off + 1] = color.g();
            self.buf[off + 2] = color.r();
            self.buf[off + 3] = 0xFF;
        }
    }

    pub fn text(&mut self, s: &str, x: i32, y: i32, color: u32) {
        let style = MonoTextStyle::new(
            &FONT_6X13,
            Rgb888::new(
                ((color >> 16) & 0xFF) as u8,
                ((color >> 8) & 0xFF) as u8,
                (color & 0xFF) as u8,
            ),
        );
        let mut target = TextTarget { canvas: self };
        let _ = Text::with_baseline(s, Point::new(x, y), style, Baseline::Top).draw(&mut target);
    }

    pub fn text_width(s: &str) -> u32 {
        // FONT_6X13 is fixed 6px advance.
        s.chars().count() as u32 * 6
    }
}

struct TextTarget<'c, 'a> {
    canvas: &'c mut Canvas<'a>,
}

impl DrawTarget for TextTarget<'_, '_> {
    type Color = Rgb888;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(Point { x, y }, color) in pixels {
            if x >= 0 && y >= 0 {
                self.canvas.put_rgb(x as u32, y as u32, color);
            }
        }
        Ok(())
    }
}

impl OriginDimensions for TextTarget<'_, '_> {
    fn size(&self) -> Size {
        Size::new(self.canvas.width, self.canvas.height)
    }
}

/// Draws the whole bar. Returns workspace click cells.
pub fn draw_bar(
    canvas: &mut Canvas<'_>,
    config: &BarConfig,
    snapshot: &CompositorSnapshot,
    clock_text: &str,
) -> Vec<WorkspaceCell> {
    canvas.fill(config.background);
    let mut cells = Vec::new();
    let h = canvas.height;

    let has_ws = config.widgets.contains(&Widget::Workspaces);
    let has_clock = config.widgets.contains(&Widget::Clock);

    // Left: workspace cells.
    let mut x = 4u32;
    if has_ws {
        for (idx, ws) in snapshot.workspaces.iter().enumerate() {
            let label = ws.name.clone();
            let label_w = Canvas::text_width(&label);
            let cell_w = (label_w + 16).max(h);
            if x + cell_w > canvas.width {
                break;
            }
            if ws.active {
                canvas.fill_rect(x, 0, cell_w, h, config.accent);
                let tx = x + (cell_w - label_w) / 2;
                canvas.text(&label, tx as i32, 7, 0xFF_FF_FF_FF);
            } else {
                let color = if ws.window_count > 0 {
                    config.foreground
                } else {
                    config.inactive
                };
                let tx = x + (cell_w - label_w) / 2;
                canvas.text(&label, tx as i32, 7, color);
                // Underline workspaces that hold windows.
                if ws.window_count > 0 {
                    canvas.fill_rect(x + 4, h - 4, cell_w - 8, 2, config.inactive);
                }
            }
            cells.push(WorkspaceCell { x0: x, x1: x + cell_w, index: idx });
            x += cell_w + 2;
        }
    }

    // Right: clock.
    if has_clock {
        let pad = 8u32;
        let clock_w = Canvas::text_width(clock_text);
        if clock_w + pad * 2 < canvas.width.saturating_sub(x) {
            let tx = canvas.width - clock_w - pad;
            canvas.text(clock_text, tx as i32, 7, config.foreground);
        }
    }

    cells
}

pub fn current_clock() -> String {
    chrono::Local::now().format("%H:%M").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aerowm_ipc::WorkspaceInfo;

    fn test_snapshot() -> CompositorSnapshot {
        CompositorSnapshot {
            active: 0,
            workspaces: vec![
                WorkspaceInfo {
                    name: "1".into(),
                    active: true,
                    window_count: 2,
                    focused_window: Some(1),
                },
                WorkspaceInfo {
                    name: "2".into(),
                    active: false,
                    window_count: 0,
                    focused_window: None,
                },
            ],
        }
    }

    #[test]
    fn draws_workspace_cells() {
        let config = BarConfig::default();
        let snap = test_snapshot();
        let mut buf = vec![0u8; 400 * 28 * 4];
        let mut canvas = Canvas::new(&mut buf, 400, 28);
        let cells = draw_bar(&mut canvas, &config, &snap, "12:00");
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].index, 0);
        // Background was painted (opaque).
        assert_eq!(buf[3], 0xFF);
    }

    #[test]
    fn text_width_counts_glyphs() {
        assert_eq!(Canvas::text_width("12:00"), 5 * 6);
    }
}
