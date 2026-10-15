use std::sync::Mutex;
use wayland_server::Resource;

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                Kind,
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                render_elements,
                surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
            },
            gles::GlesRenderer,
        },
    },
    desktop::space::SpaceRenderElements,
    input::pointer::{CursorImageAttributes, CursorImageStatus},
    utils::{Logical, Point, Rectangle, Scale, Transform},
};

render_elements! {
    pub CustomRenderElement<=GlesRenderer>;
    Space=SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>,
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
}

const CURSOR_WIDTH: i32 = 24;
const CURSOR_HEIGHT: i32 = 24;

const CURSOR_PIXELS: [&str; 24] = [
    "B                       ",
    "BB                      ",
    "BWB                     ",
    "BWWB                    ",
    "BWWWB                   ",
    "BWWWWB                  ",
    "BWWWWWB                 ",
    "BWWWWWWB                ",
    "BWWWWWWWB               ",
    "BWWWWWWWWB              ",
    "BWWWWWWWWWB             ",
    "BWWWWWWWWWWB            ",
    "BWWWWWWWWWWWB           ",
    "BWWWWWWWWWWWWB          ",
    "BWWWWWWWWWWWWWB         ",
    "BWWWWWWBBBBBBBB         ",
    "BWWWBWWWB               ",
    "BWWB BWWWB              ",
    "BWB   BWWWB             ",
    "BB     BWWWB            ",
    "B       BWWWB           ",
    "         BWWWB          ",
    "         BWWWB          ",
    "          BBB           ",
];

/// Creates a default software-rendered cursor buffer (classic 24x24 arrow).
pub fn create_default_cursor() -> MemoryRenderBuffer {
    let mut buffer = MemoryRenderBuffer::new(
        Fourcc::Abgr8888,
        (CURSOR_WIDTH, CURSOR_HEIGHT),
        1,
        Transform::Normal,
        None,
    );

    let _ = buffer.render().draw(|buf| {
        for (y, row) in CURSOR_PIXELS.iter().enumerate() {
            for (x, ch) in row.chars().enumerate() {
                if x >= CURSOR_WIDTH as usize || y >= CURSOR_HEIGHT as usize {
                    continue;
                }
                let idx = (y * CURSOR_WIDTH as usize + x) * 4;
                let color: [u8; 4] = match ch {
                    'B' => [0, 0, 0, 255],
                    'W' => [255, 255, 255, 255],
                    _ => [0, 0, 0, 0],
                };
                buf[idx..idx + 4].copy_from_slice(&color);
            }
        }
        Ok::<_, ()>(vec![Rectangle::new(
            (0, 0).into(),
            (CURSOR_WIDTH, CURSOR_HEIGHT).into(),
        )])
    });

    buffer
}

/// Generates cursor render elements according to current seat cursor status.
pub fn cursor_render_elements(
    cursor_status: &CursorImageStatus,
    pointer_location: Point<f64, Logical>,
    default_cursor: &MemoryRenderBuffer,
    renderer: &mut GlesRenderer,
    scale: Scale<f64>,
) -> Vec<CustomRenderElement> {
    match cursor_status {
        CursorImageStatus::Hidden => Vec::new(),
        CursorImageStatus::Surface(wl_surface) => {
            if !wl_surface.is_alive() {
                return render_default_cursor(pointer_location, default_cursor, renderer, scale);
            }
            let hotspot = smithay::wayland::compositor::with_states(wl_surface, |states| {
                states
                    .data_map
                    .get::<Mutex<CursorImageAttributes>>()
                    .and_then(|h| h.lock().ok().map(|h| h.hotspot))
                    .unwrap_or_default()
            });
            let cursor_pos =
                (pointer_location - hotspot.to_f64()).to_physical_precise_round(scale);
            render_elements_from_surface_tree::<_, WaylandSurfaceRenderElement<GlesRenderer>>(
                renderer,
                wl_surface,
                cursor_pos,
                scale,
                1.0,
                Kind::Cursor,
            )
            .into_iter()
            .map(CustomRenderElement::Surface)
            .collect()
        }
        CursorImageStatus::Named(_) => {
            render_default_cursor(pointer_location, default_cursor, renderer, scale)
        }
    }
}

fn render_default_cursor(
    pointer_location: Point<f64, Logical>,
    default_cursor: &MemoryRenderBuffer,
    renderer: &mut GlesRenderer,
    scale: Scale<f64>,
) -> Vec<CustomRenderElement> {
    let cursor_pos = pointer_location.to_physical_precise_round(scale);
    match MemoryRenderBufferRenderElement::from_buffer(
        renderer,
        cursor_pos,
        default_cursor,
        None,
        None,
        None,
        Kind::Cursor,
    ) {
        Ok(elem) => vec![CustomRenderElement::Memory(elem)],
        Err(e) => {
            tracing::warn!("Failed to create default cursor element: {e:?}");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_default_cursor() {
        let _cursor = create_default_cursor();
    }
}

