use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture, GlesTarget};
use smithay::backend::renderer::ExportMem;
use smithay::backend::allocator::format::Fourcc;
use smithay::utils::{Rectangle, Size};

fn test(renderer: &mut GlesRenderer, framebuffer: &GlesTarget<'_>, size: Size<i32, smithay::utils::BufferCoord>) {
    let rect = smithay::utils::Rectangle::from_loc_and_size((0, 0), size);
    if let Ok(mapping) = renderer.copy_framebuffer(framebuffer, rect, Fourcc::Abgr8888) {
        if let Ok(data) = renderer.map_texture(&mapping) {
            let data_vec = data.to_vec();
            let w = size.w as u32;
            let h = size.h as u32;
            std::thread::spawn(move || {
                let path = "/tmp/screenshot.png";
                if let Some(buf) = image::RgbaImage::from_raw(w, h, data_vec) {
                    let _ = image::save_buffer(path, &buf, w, h, image::ColorType::Rgba8);
                }
            });
        }
    }
}
