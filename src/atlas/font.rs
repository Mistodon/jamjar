use image::RgbaImage;
use rusttype::gpu_cache::Cache;

use crate::{atlas::Atlas, draw::GlyphRegion, font::Glyph};

pub struct FontAtlas {
    glyph_cache: Cache<'static>,
    backing_image_size: [u32; 2],
    available_area: ([u32; 2], [u32; 2]),
    modified: bool,
}

impl FontAtlas {
    pub fn new() -> Self {
        Self::with_size([4096, 4096])
    }

    pub fn with_size(backing_size: [u32; 2]) -> Self {
        Self::with_area_in_size(([0, 0], backing_size), backing_size)
    }

    pub fn with_area_in_size(
        (topleft, size): ([u32; 2], [u32; 2]),
        backing_size: [u32; 2],
    ) -> Self {
        FontAtlas {
            glyph_cache: Cache::builder()
                .dimensions(size[0], size[1])
                .position_tolerance(0.1)
                .scale_tolerance(0.1)
                .pad_glyphs(true)
                .multithread(true)
                .build(),
            backing_image_size: backing_size,
            available_area: (topleft, size),
            modified: true,
        }
    }

    pub fn compile(&mut self) -> RgbaImage {
        let [bw, bh] = self.backing_image_size;
        let mut atlas = RgbaImage::new(bw, bh);
        self.compile_into(&mut atlas);
        atlas
    }
}

impl Atlas<Glyph, Glyph, Option<GlyphRegion>, RgbaImage> for FontAtlas {
    fn insert(&mut self, insertion: Glyph) {
        self.glyph_cache
            .queue_glyph(insertion.font_id, insertion.glyph);
    }

    fn fetch(&self, key: &Glyph) -> Option<GlyphRegion> {
        let [bw, bh] = self.backing_image_size;
        let ([ax, ay], [aw, ah]) = self.available_area;
        let scale_u = aw as f32 / bw as f32;
        let scale_v = ah as f32 / bh as f32;
        let off_u = ax as f32 / bw as f32;
        let off_v = ay as f32 / bh as f32;

        let scale = key.glyph.scale();
        let ascent = key.glyph.font().v_metrics(scale).ascent;

        let coords = self.glyph_cache.rect_for(key.font_id, &key.glyph).unwrap();

        coords.map(|(uv_rect, px_rect)| {
            use rusttype::Point;

            let Point { x, y } = px_rect.min;
            let w = px_rect.width() as f32;
            let h = px_rect.height() as f32;

            let Point { x: u, y: v } = uv_rect.min;
            let uw = uv_rect.width();
            let vh = uv_rect.height();
            let uv = (
                [u * scale_u + off_u, v * scale_v + off_v],
                [uw * scale_u, vh * scale_v],
            );

            GlyphRegion {
                pos: [x as f32, y as f32 + ascent],
                size: [w, h],
                uv,
            }
        })
    }

    fn compile_into(&mut self, dest: &mut RgbaImage) -> bool {
        let mut upload_required = false;

        let ([ax, ay], _) = self.available_area;

        self.glyph_cache
            .cache_queued(|dest_rect, data| {
                use rusttype::Point;

                let Point { x, y } = dest_rect.min;
                let w = dest_rect.width();
                let h = dest_rect.height();
                for dy in 0..h {
                    for dx in 0..w {
                        let alpha = data[(dy * w + dx) as usize];
                        dest.put_pixel(x + ax + dx, y + ay + dy, [255, 255, 255, alpha].into());
                    }
                }

                upload_required = true;
            })
            .unwrap();

        self.modified = true;
        upload_required
    }

    fn modified(&self) -> bool {
        self.modified
    }
}
